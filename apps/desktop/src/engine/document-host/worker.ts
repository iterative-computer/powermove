/*
 * One document host worker: one project in a process of its own
 * (docs/engine/document-host.md). It boots the editor's document chain
 * (chain.ts), loads the project from the session, resumes the undo history
 * it persisted, answers tool calls, and sends its edits back to the session
 * before each reply, so the host's copy is never behind an answer the agent
 * holds (contract 15, invariant 3).
 *
 * The pool talks to it over child-process IPC (protocol.ts `ToWorker` /
 * `FromWorker`); every host service it needs is a `service` request.
 */
import type { AgentToolRequestEvent, AgentToolResponseEvent, PowermoveBridge } from '../../shared/ipc';
import { diffPatches, applyPatch, type Patch } from '../../shared/patch';
import { canonicalProjectHash } from '../../shared/canonical-json';
import { NEWER_PROJECT_MESSAGE, needsNewerReader } from '../../shared/reader-version';
import { REQUIRES_NATIVE_MESSAGE, refusesProject } from '../../shared/engine-stamp';
import type { ExtensionRecord } from '../../shared/extensions';
import type { PersistReason } from '../../shared/engine-protocol';
import type { TranscribeProgress } from '../../shared/transcription';
import { bootChain, memoryStore, type Chain, type EnginePolicy } from './chain';
import { deterministicSeam } from './contract';
import { recordFor } from '../../renderer/src/kernel/extensions.svelte';
import { BUILTIN_EXTENSIONS } from '../../renderer/src/kernel/builtins';
import { WorkerExtensions } from './extensions';
import {
  DOCUMENT_HOST_PROTOCOL, DocumentHostError, INSTANCE_STATE_KEY, isFailure,
  type DocumentHostFailure, type FromWorker, type InstanceOpened, type InstancePersisted, type InstanceStats,
  type ExtensionRegistration, type ServiceName, type SessionPatch, type ToWorker,
} from './protocol';

/** What a worker persists per instance (`documentHost.<projectId>` in the profile store). */
export interface InstanceState {
  v: 1;
  projectId: string;
  /** The document's canonical hash when the history was saved. */
  hash: string;
  revision: number;
  /** `PM.hist.export()`. */
  history: unknown;
  savedAt: number;
}

/** A transport to the pool: `process` in a forked child, a pair of functions in tests. */
export interface WorkerPort {
  send(message: FromWorker): void;
  onMessage(listener: (message: ToWorker) => void): void;
}

export class DocumentWorker {
  private chain: Chain | null = null;
  private projectId: string | null = null;
  private instance = 0;
  private synced = '';
  private seq = 0;
  private definitionsGeneration = 0;
  private readOnly = false;
  private nextService = 1;
  private services = new Map<number, { resolve(v: unknown): void; reject(e: Error): void }>();
  private progress = new Set<(p: TranscribeProgress) => void>();
  private toolResponses = new Map<string, (response: AgentToolResponseEvent) => void>();
  private onRequest: ((request: AgentToolRequestEvent) => void) | null = null;
  private extensions = { active: [] as string[], failed: [] as Array<{ id: string; error: string }> };
  private queue: Promise<unknown> = Promise.resolve();
  private readonly ext: WorkerExtensions;

  constructor(private readonly port: WorkerPort, private readonly policy: EnginePolicy) {
    port.onMessage((message) => this.receive(message));
    this.ext = new WorkerExtensions({
      list: () => this.service<ExtensionRecord[] | null>('extensions.list'),
      bundle: (url) => this.service<string>('extensions.bundle', url),
      values: (id) => this.service<Record<string, string>>('extensions.values', id),
      log: (line) => this.log(line),
    }, Object.keys(BUILTIN_EXTENSIONS));
  }

  start(): void {
    this.port.send({ t: 'ready', protocol: DOCUMENT_HOST_PROTOCOL, pid: process.pid });
  }

  private log(line: string): void { this.port.send({ t: 'log', line }); }

  private service<T>(name: ServiceName, ...args: unknown[]): Promise<T> {
    const id = this.nextService++;
    return new Promise<T>((resolve, reject) => {
      this.services.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.port.send({ t: 'service', id, name, args });
    });
  }

  private receive(message: ToWorker): void {
    switch (message.t) {
      case 'serviceReply': {
        const pending = this.services.get(message.id);
        if (!pending) return;
        this.services.delete(message.id);
        if (message.ok) pending.resolve(message.value); else pending.reject(new Error(message.error ?? 'host service failed'));
        return;
      }
      case 'progress':
        for (const listener of this.progress) listener(message.progress);
        return;
      case 'remote':
        this.applyRemote(message.patch);
        return;
      case 'extensionsChanged':
        // The kernel's loader reloads what changed, in its own queue, as in the editor.
        this.ext.emit(message.event);
        void this.chain?.loader()?.whenIdle().then(() => { this.extensions = this.extensionState(); });
        return;
      case 'tool':
        // Tool calls run concurrently (the harness keeps one transaction per run).
        void this.answer(message.id, () => this.tool(message.request));
        return;
      default:
        // Lifecycle requests run one at a time, in order.
        this.queue = this.queue.then(() => this.answer(message.id, () => this.lifecycle(message)));
    }
  }

  private async answer(id: number, run: () => Promise<unknown>): Promise<void> {
    try {
      const value = await run();
      this.port.send({ t: 'reply', id, ok: true, value });
    } catch (error) {
      const failure: DocumentHostFailure = error instanceof DocumentHostError ? error.toJSON()
        : isFailure(error) ? error : { code: 'worker-error', message: error instanceof Error ? error.message : String(error), retry: false };
      this.port.send({ t: 'reply', id, ok: false, failure });
    }
  }

  private lifecycle(message: Exclude<ToWorker, { t: 'tool' | 'remote' | 'serviceReply' | 'progress' | 'extensionsChanged' }>): Promise<unknown> {
    switch (message.t) {
      case 'open': return this.open(message.projectId, message.instance);
      case 'persist': return this.persist(message.reason);
      case 'register': return this.register(message.record);
      case 'stats': return Promise.resolve(this.stats());
      case 'close': return this.close();
    }
  }

  /* ── the bridge the editor's modules see ─────────────────────────────── */

  private bridge(): Partial<PowermoveBridge> {
    const agentTools: PowermoveBridge['agentTools'] = {
      onRequest: (cb) => { this.onRequest = cb; return () => { if (this.onRequest === cb) this.onRequest = null; }; },
      respond: (response) => {
        const key = `${response.runId}\u0000${response.callId}`;
        const done = this.toolResponses.get(key);
        if (done) { this.toolResponses.delete(key); done(response); }
      },
    };
    const unavailable = (what: string) => () => Promise.reject(new Error(`${what} is not available in the document host.`));
    return {
      agentTools,
      // The user's extensions through the kernel's own loader (extensions.ts).
      ...(this.ext.available ? { extensions: this.ext.bridge(), vars: this.ext.vars() as never } : {}),
      remote: true,
      versions: { electron: '', chrome: '', node: process.versions.node },
      log: (level: string, text: string) => this.log(`[engine:${level}] ${text}`),
      mediaPath: { lookup: (request) => this.service('mediaPath.lookup', request) },
      transcription: {
        status: () => this.service('transcription.status'),
        transcribe: (request) => this.service('transcription.transcribe', request),
        cancelTranscribe: (requestId) => this.service('transcription.cancelTranscribe', requestId),
        onProgress: (callback) => { this.progress.add(callback); return () => { this.progress.delete(callback); }; },
        onStatus: () => () => {},
        onModelRequested: () => () => {},
        download: unavailable('Downloading speech models'),
        cancelDownload: unavailable('Downloading speech models'),
        remove: unavailable('Removing speech models'),
        setActive: unavailable('Choosing a speech model'),
        setLanguage: unavailable('Choosing a transcription language'),
        reveal: unavailable('Showing the models folder'),
      },
    } as Partial<PowermoveBridge>;
  }

  /* ── lifecycle ───────────────────────────────────────────────────────── */

  private async open(projectId: string, instance: number): Promise<InstanceOpened> {
    if (this.projectId && this.projectId !== projectId) throw new DocumentHostError('worker-error', `This worker holds project ${this.projectId}.`);
    if (this.chain && this.projectId === projectId) return this.opened(0);
    const joined = await this.service<{ seq: number; doc: unknown | null }>('session.join', projectId);
    if (!joined?.doc) throw new DocumentHostError('project-missing', `The host has no document for project ${projectId}.`);
    // The shared reader guards (plan sections 6 and 21), as the desktop applies them.
    if (refusesProject(joined.doc, this.policy.newEngine)) throw new DocumentHostError('requires-new-engine', `${REQUIRES_NATIVE_MESSAGE}`);
    this.readOnly = needsNewerReader(joined.doc);
    if (this.readOnly) this.log(`[engine] ${projectId}: ${NEWER_PROJECT_MESSAGE}. It is open read-only, so changes are not saved.`);
    const saved = await this.service<InstanceState | null>('store.read', INSTANCE_STATE_KEY(projectId)).catch(() => null);
    await this.ext.probe();
    this.chain = await bootChain({ bridge: this.bridge(), store: memoryStore(), log: (line) => this.log(line), policy: this.policy });
    const PM = this.chain.PM;
    if (!this.onRequest) throw new DocumentHostError('worker-error', 'The agent tool harness did not register.');
    this.projectId = projectId;
    this.instance = instance;
    this.seq = joined.seq;
    PM['isHomeProject'] = () => !PM['proj']?.id;
    PM['replaceProject'](joined.doc, {});
    PM['bus']?.emit('projects:open');
    this.synced = JSON.stringify(PM['proj']);
    // The undo history this instance had: never cleared by another project,
    // resumed after a restart (amendment: appendix A item 16).
    let restored = 0;
    if (saved && saved.v === 1 && saved.projectId === projectId && saved.history) {
      if (PM['hist'].import(saved.history)) restored = PM['hist'].export({ copy: false }).entries.length;
      if (saved.hash !== canonicalProjectHash(PM['proj'])) this.log(`[engine] ${projectId}: the document changed since its history was saved (another member edited it); undo applies on top`);
    }
    await this.chain.bootExtensions();
    this.extensions = this.extensionState();
    // The contract tests' deterministic ids and clock (contract.ts), tests only.
    if (process.env['POWERMOVE_DOCUMENT_HOST_TEST_SEED']) deterministicSeam();
    this.log(`[engine] opened ${projectId} (seq ${joined.seq}, ${restored} undo entries, extensions ${this.extensions.active.length} active)`);
    return this.opened(restored);
  }

  private opened(restored: number): InstanceOpened {
    const PM = this.chain!.PM;
    return {
      instance: this.instance,
      projectId: this.projectId!,
      document: this.projectId!,
      revision: Number(PM['proj']?.revision) || 0,
      restoredHistory: restored,
      readOnly: this.readOnly,
      engine: { evaluator: 'pm-wasm', mode: this.chain!.engine.mode, fonts: this.chain!.engine.fonts },
    };
  }

  private requireChain(): Chain {
    if (!this.chain || !this.projectId) throw new DocumentHostError('closed', 'The document host instance is not open.');
    return this.chain;
  }

  private async persist(reason: PersistReason): Promise<InstancePersisted> {
    const PM = this.requireChain().PM;
    if (this.readOnly) return { instance: this.instance, projectId: this.projectId!, revision: Number(PM['proj']?.revision) || 0, bytes: 0, reason };
    await this.syncUp();
    const state: InstanceState = {
      v: 1,
      projectId: this.projectId!,
      hash: canonicalProjectHash(PM['proj']),
      revision: Number(PM['proj']?.revision) || 0,
      history: PM['hist'].export({ copy: false }),
      savedAt: Date.now(),
    };
    const bytes = Buffer.byteLength(JSON.stringify(state));
    await this.service('store.write', INSTANCE_STATE_KEY(this.projectId!), state);
    return { instance: this.instance, projectId: this.projectId!, revision: state.revision, bytes, reason };
  }

  private async close(): Promise<null> {
    if (!this.chain) return null;
    await this.persist('close');
    await this.service('session.leave', this.projectId);
    await this.chain.loader()?.dispose();
    this.ext.dispose();
    return null;
  }

  /** Active ids, and the extensions whose health is not ok (the worker's own). */
  private extensionState(): { active: string[]; failed: Array<{ id: string; error: string }> } {
    const loader = this.chain?.loader();
    if (!loader) return { active: [], failed: [] };
    const failed = loader.records()
      .filter((r) => r.health && !['ok', 'disabled', 'replaced'].includes(r.health.state))
      .map((r) => ({ id: r.id, error: 'error' in r.health && r.health.error ? String(r.health.error) : r.health.state }));
    return { active: loader.activeIds(), failed };
  }

  /**
   * Registers (or hot-reloads) a record through the loader's own path, as an
   * `extensionsChanged` for that id: `planLoad` decides (a disabled record
   * stays off, a fork replaces its built-in, a missing dependency or a
   * blocked state skips it), and a Store record runs in the contained
   * sandbox. Resolves once the loader settled, so the next call sees its
   * contributions (contract 15, invariant 4).
   */
  private async register(record: ExtensionRecord): Promise<ExtensionRegistration> {
    const loader = this.requireChain().loader();
    if (!loader) throw new DocumentHostError('worker-error', 'The extension loader has not booted.');
    if (!this.ext.available) throw new DocumentHostError('service-unavailable', 'This host gives document hosts no extensions service, so extensions cannot be registered.');
    this.ext.register(record);
    this.ext.emit({ ids: [record.id], reason: 'reload' });
    await loader.whenIdle();
    this.extensions = this.extensionState();
    const health = recordFor(record.id)?.health as { state?: string; error?: unknown } | undefined;
    const active = loader.activeIds().includes(record.id);
    if (!active && (health?.state === 'activation-error' || health?.state === 'runtime-error')) {
      throw new DocumentHostError('worker-error', `Extension ${record.id} failed to activate: ${health.error ? String(health.error) : health.state}`);
    }
    this.definitionsGeneration += 1;
    return { instance: this.instance, extension: record.id, definitionsGeneration: this.definitionsGeneration, active };
  }

  private stats(): InstanceStats {
    const PM = this.requireChain().PM;
    const engine = PM['EngineDoc']?.stats?.() ?? {};
    const fallbacks: Record<string, number> = {};
    for (const [k, v] of Object.entries(engine.native?.fallbacks ?? {})) fallbacks[k] = (fallbacks[k] ?? 0) + Number(v);
    for (const [k, v] of Object.entries(engine.edits?.fallbacks ?? {})) fallbacks[`edit.${k}`] = (fallbacks[`edit.${k}`] ?? 0) + Number(v);
    const calls = Object.values(engine.native?.calls ?? {}).reduce((a: number, b) => a + Number(b), 0) + Object.values(engine.edits?.calls ?? {}).reduce((a: number, b) => a + Number(b), 0);
    return {
      instance: this.instance,
      projectId: this.projectId!,
      revision: Number(PM['proj']?.revision) || 0,
      hash: canonicalProjectHash(PM['proj']),
      historyDepth: PM['hist'].export({ copy: false }).entries.length,
      fallbacks,
      calls,
      auditor: engine.audit ? { audits: engine.audit.audits, failures: engine.audit.failures } : null,
      dual: engine.dual ? { compared: sum(engine.dual.compared), mismatched: sum(engine.dual.mismatched), stale: sum(engine.dual.stale), mismatchedOps: { ...engine.dual.mismatched } } : null,
      failures: (PM['EngineDoc']?.failures?.() ?? []).map((f: { code: string; message: string; retry: boolean }) => ({ code: f.code, message: f.message, retry: f.retry })),
      consistency: { checks: Number(engine.barrier?.checks ?? 0), mismatches: Number(engine.barrier?.checkMismatches ?? 0) },
      extensions: this.extensions,
    };
  }

  /* ── tools ───────────────────────────────────────────────────────────── */

  private async tool(request: AgentToolRequestEvent): Promise<AgentToolResponseEvent> {
    const PM = this.requireChain().PM;
    const handle = this.onRequest;
    if (!handle) throw new DocumentHostError('worker-error', 'The agent tool harness is not registered.');
    const key = `${request.runId}\u0000${request.callId}`;
    if (this.toolResponses.has(key)) throw new DocumentHostError('worker-error', `Tool call ${request.callId} of run ${request.runId} is already running.`);
    const response = await new Promise<AgentToolResponseEvent>((resolve) => {
      this.toolResponses.set(key, resolve);
      handle(request);
    });
    // Read-your-writes across the hop: the session has this call's edits
    // before the agent sees the answer.
    await this.syncUp();
    // The production consistency check (plan section 6, item 7) after every
    // call that changed the document: the mirror must hash as the document.
    if (Number(PM['proj']?.revision) !== request.baseRevision) PM['EngineDoc']?.check?.('agent-tool');
    if (response.changed || Number(PM['proj']?.revision) !== request.baseRevision) await this.persist('toolCall').catch((error) => this.log(`[engine] persist after ${request.tool} failed: ${(error as Error).message}`));
    return response;
  }

  /* ── the session ─────────────────────────────────────────────────────── */

  /** The host's patch sends, in order; never rejects (each send reports to its own caller). */
  private sending: Promise<void> = Promise.resolve();
  /** A send the host refused: the next sync reconciles with the host's copy. */
  private diverged = false;
  /** Batches queued or in flight that the host has not acknowledged. */
  private unacked = new Set<Patch[]>();
  /** A reconciliation with the host's copy, queued behind the sends; it sends every local edit. */
  private reconciling: Promise<void> | null = null;

  /**
   * Diffs the document against what the session last had from us, marks it
   * synced, and queues the patches, all synchronously, as remote-sync does in
   * a tab: whatever is in the document now is in this batch, so an edit made
   * while an earlier batch or a remote patch is in flight is never marked
   * synced without being sent. Resolves once the host applied this batch (and
   * every batch before it); rejects when the host refused it.
   */
  private flushLocal(): Promise<void> {
    if (!this.chain || !this.projectId || this.readOnly) return this.sending;
    // A pending reconciliation sends everything the document has when it runs.
    if (this.reconciling) return this.reconciling;
    if (this.diverged) { this.diverged = false; return this.startReconcile(); }
    const projectId = this.projectId;
    const after = JSON.stringify(this.chain.PM['proj']);
    if (after === this.synced) return this.sending;
    const patches: Patch[] = diffPatches(JSON.parse(this.synced), JSON.parse(after)).forward;
    this.synced = after;
    if (!patches.length) return this.sending;
    this.unacked.add(patches);
    const send = this.sending.then(async () => {
      try { this.seq = await this.service<number>('session.patch', projectId, patches); } finally { this.unacked.delete(patches); }
    });
    this.sending = send.catch((error) => {
      this.diverged = true;
      this.log(`[engine] ${projectId}: the host refused an edit batch (${(error as Error)?.message ?? error}); the next sync sends it again`);
    });
    return send;
  }

  /**
   * Behind every queued send: join again (idempotent for a member), take the
   * host's copy, put this document's unsent edits on top of it, and send
   * them. Used after a refused batch, and after a remote patch that wrote a
   * path one of our unacknowledged batches also writes, where the host's
   * order (which this worker cannot see) decides the value. Until it has run,
   * no new batch is cut (`flushLocal` answers this promise) and remote
   * patches advance `synced` and the document alike, so `synced` stays what
   * the host had from us.
   */
  private startReconcile(): Promise<void> {
    const projectId = this.projectId!;
    const run = this.sending.then(async () => {
      try {
        const joined = await this.service<{ seq: number; doc: unknown | null }>('session.join', projectId);
        if (!this.chain || this.projectId !== projectId || !joined?.doc) return;
        const PM = this.chain.PM;
        const local = diffPatches(JSON.parse(this.synced), JSON.parse(JSON.stringify(PM['proj']))).forward;
        const host = JSON.stringify(joined.doc);
        this.seq = joined.seq;
        const next = local.length ? applyPatch(JSON.parse(host), local) : JSON.parse(host);
        if (JSON.stringify(next) !== JSON.stringify(PM['proj'])) PM['replaceProject'](next, { selection: PM['sel'] });
        const mine = diffPatches(JSON.parse(host), JSON.parse(JSON.stringify(PM['proj']))).forward;
        this.synced = JSON.stringify(PM['proj']);
        if (mine.length) this.seq = await this.service<number>('session.patch', projectId, mine);
      } finally {
        this.reconciling = null;
      }
    });
    this.reconciling = run;
    this.sending = run.catch((error) => {
      this.diverged = true;
      this.log(`[engine] ${projectId}: reconciling with the host failed (${(error as Error)?.message ?? error}); the next sync tries again`);
    });
    return run;
  }

  /** Sends what changed since the last sync; resolves once the host applied it. */
  private async syncUp(): Promise<void> {
    try {
      await this.flushLocal();
    } catch (error) {
      throw new DocumentHostError('service-unavailable', `The host did not take this call's edits: ${(error as Error)?.message ?? error}. They stay in the document host and are sent with the next call.`, true);
    }
  }

  private applyRemote(patch: SessionPatch): void {
    if (!this.chain || patch.projectId !== this.projectId) return;
    const PM = this.chain.PM;
    // Our own unsent edits are queued first (now, synchronously), then the
    // remote patch applies at once, as remote-sync does in a tab.
    void this.flushLocal().catch(() => { /* reported by flushLocal; resent on the next sync */ });
    // Patches on disjoint paths commute. When the remote patch writes a path
    // one of our unacknowledged batches also writes, the host's order decides
    // the value: reconcile with its copy once our sends have landed.
    const conflicts = [...this.unacked].some((batch) => batch.some((mine) => patch.patches.some((theirs) => overlaps(mine.path, theirs.path))));
    // What the host has from us, and the document, both take the patch.
    this.synced = JSON.stringify(applyPatch(JSON.parse(this.synced), structuredClone(patch.patches)));
    PM['replaceProject'](applyPatch(PM['proj'], patch.patches), { selection: PM['sel'] });
    if (!this.reconciling) this.synced = JSON.stringify(PM['proj']);
    this.seq = patch.seq;
    if (conflicts && !this.reconciling) void this.startReconcile().catch(() => { /* logged; retried on the next sync */ });
  }
}

/** One path is a prefix of the other: the two writes touch the same value. */
const overlaps = (a: readonly unknown[], b: readonly unknown[]): boolean => {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) if (String(a[i]) !== String(b[i])) return false;
  return true;
};

const sum = (table: Record<string, number> | undefined): number => Object.values(table ?? {}).reduce((a, b) => a + Number(b), 0);

/* child-process entry: forked by the pool with `--document-worker` */
export function runWorkerProcess(): void {
  if (typeof process.send !== 'function') throw new Error('The document worker must be forked with an IPC channel.');
  const policy: EnginePolicy = { newEngine: process.env['POWERMOVE_DOCUMENT_HOST_NEW_ENGINE'] === '1', evaluator: process.env['POWERMOVE_NATIVE_EVALUATOR'] };
  const port: WorkerPort = {
    send: (message) => { process.send!(message); },
    onMessage: (listener) => { process.on('message', (message) => listener(message as ToWorker)); },
  };
  // The pool owns this process: when its channel closes, the project is gone.
  process.on('disconnect', () => process.exit(0));
  new DocumentWorker(port, policy).start();
}
