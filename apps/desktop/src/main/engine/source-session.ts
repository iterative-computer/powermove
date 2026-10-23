/** Bounded source sessions. A session's source bytes never change; its revision may advance. */
import { randomUUID } from 'node:crypto';
import { canonicalHash, canonicalProject, canonicalStringify } from '../../shared/canonical-json';
import type { EngineMediaJobRequest } from '../../shared/engine-ipc';
import { STANDARD_COLOR, type AgentJobsEngine } from './agent-jobs';

const INLINE_BYTES = 4 * 1024 * 1024;
const MAX_PER_OWNER = 4;
const MAX_TOTAL = 16;
const IDLE_MS = 90_000;

type Entry = {
  owner: number;
  asset: string;
  revision: number;
  key: string;
  project: Record<string, any>;
  document: string;
  body: Uint8Array;
  hash: Uint8Array;
  ready: Promise<number>;
  sync: Promise<void>;
  session: number | null;
  refs: number;
  invalid: boolean;
  abort: AbortController;
  idle: ReturnType<typeof setTimeout> | null;
  closing: Promise<void> | null;
  lastUsed: number;
  warmed: boolean;
  phases: { snapshotMs: number; waitMs: number; openMs: number; bindMs: number; resyncMs: number };
};

/** Only fields a source job reads. Poster is a generated UI artifact. */
export function sourceOnlyProject(request: EngineMediaJobRequest): Record<string, any> {
  const parsed = JSON.parse(request.project);
  const project = parsed.proj ?? parsed;
  const meta = project.assets?.[request.asset];
  if (!meta || typeof meta !== 'object' || Array.isArray(meta)) throw new Error('The source is not in the frozen project.');
  const { poster: _poster, ...source } = meta;
  return { id: project.id, assets: { [request.asset]: source } };
}

/** Used only to join overlapping prepares/strips before main pins media. */
export function sourceInputKey(request: EngineMediaJobRequest): string {
  const key = canonicalStringify(sourceOnlyProject(request));
  if (key === undefined) throw new Error('The source project could not be serialized.');
  return key;
}

type Options = {
  engine: AgentJobsEngine;
  snapshotMedia?(project: Record<string, any>): Promise<Record<string, any>>;
  bindMediaSession?(session: number, project: Record<string, any> | null): Promise<void>;
};

export class SourceSessionPool {
  private readonly owners = new Map<number, Map<string, Entry>>();
  private readonly ownerEpochs = new Map<number, { epoch: number; pending: number }>();
  private globalEpoch = 0;
  private disposed = false;
  private ready = false;
  /** A restarted engine may reuse numeric session ids; old bindings retire first. */
  private restartBarrier: Promise<void> = Promise.resolve();
  private readonly unlisten: () => void;

  constructor(private readonly options: Options) {
    this.ready = options.engine.isReady('job');
    this.unlisten = options.engine.listen({ status: () => {
      const ready = options.engine.isReady('job');
      if (this.ready && !ready) this.invalidateAll(true);
      this.ready = ready;
    } });
  }

  async acquire(owner: number, request: EngineMediaJobRequest): Promise<{ session: number; reused: boolean; warmed: boolean; phases: Entry['phases']; release(): void }> {
    if (this.disposed) throw new Error('The job engine is not running.');
    this.verifyLaunch();
    let ownerState = this.ownerEpochs.get(owner);
    if (!ownerState) this.ownerEpochs.set(owner, ownerState = { epoch: 0, pending: 0 });
    const ownerEpoch = ownerState.epoch;
    const globalEpoch = this.globalEpoch;
    // Snapshot only the requested source before waiting for a cold engine.
    // This also catches a mutable storage key replaced without a document edit.
    const frozenAt = performance.now();
    const source = sourceOnlyProject(request);
    ownerState.pending++;
    let project: Record<string, any>;
    try { project = await this.options.snapshotMedia?.(source) ?? source; }
    finally {
      ownerState.pending--;
      if (!ownerState.pending && this.ownerEpochs.get(owner) === ownerState) this.ownerEpochs.delete(owner);
    }
    this.verifyLaunch();
    if (this.disposed || globalEpoch !== this.globalEpoch || ownerEpoch !== ownerState.epoch)
      throw new Error('The source session was invalidated.');
    const canonical = canonicalProject(project);
    const key = canonicalStringify(canonical);
    if (key === undefined) throw new Error('The source project could not be serialized.');
    const snapshotMs = performance.now() - frozenAt;
    let entries = this.owners.get(owner);
    if (!entries) this.owners.set(owner, entries = new Map());
    let entry = entries.get(request.asset);
    const reused = !!entry && !entry.invalid && entry.key === key && request.revision >= entry.revision;
    if (!reused) {
      if (entry) this.invalidate(entry);
      this.makeRoom(owner);
      const body = new TextEncoder().encode(key);
      entry = {
        owner, asset: request.asset, revision: request.revision, key, project,
        document: typeof project.id === 'string' && project.id.length > 0 && new TextEncoder().encode(project.id).length <= 256
          ? project.id : `media-${randomUUID()}`,
        body, hash: Uint8Array.from(canonicalHash(canonical).match(/../g)!.map(v => parseInt(v, 16))),
        ready: Promise.resolve(-1), sync: Promise.resolve(), session: null, refs: 0, invalid: false,
        abort: new AbortController(), idle: null, closing: null, lastUsed: Date.now(), warmed: false,
        phases: { snapshotMs, waitMs: 0, openMs: 0, bindMs: 0, resyncMs: 0 }
      };
      entries = this.owners.get(owner) ?? new Map();
      this.owners.set(owner, entries);
      entries.set(request.asset, entry);
      entry.ready = this.open(entry).catch(error => {
        this.invalidate(entry!);
        throw error;
      });
    }
    entry!.refs++;
    if (entry!.idle) clearTimeout(entry!.idle);
    entry!.idle = null;
    try {
      const session = await entry!.ready;
      if (entry!.invalid) throw new Error('The source session was invalidated.');
      const resyncMs = await this.advance(entry!, request.revision);
      if (entry!.invalid) throw new Error('The source session was invalidated.');
      return { session, reused, warmed: entry!.warmed,
        phases: reused ? { snapshotMs, waitMs: 0, openMs: 0, bindMs: 0, resyncMs } : entry!.phases,
        release: () => this.release(entry!) };
    } catch (error) {
      this.release(entry!);
      throw error;
    }
  }

  markWarm(owner: number, asset: string, session: number): void {
    const entry = this.owners.get(owner)?.get(asset);
    if (entry && !entry.invalid && entry.session === session) entry.warmed = true;
  }

  invalidateSource(owner: number, asset: string, session: number): void {
    const entry = this.owners.get(owner)?.get(asset);
    if (entry && entry.session === session) this.invalidate(entry);
  }

  forget(owner: number): void {
    const state = this.ownerEpochs.get(owner);
    if (state) {
      state.epoch++;
      if (!state.pending) this.ownerEpochs.delete(owner);
    }
    for (const entry of this.owners.get(owner)?.values() ?? []) this.invalidate(entry);
    this.owners.delete(owner);
  }

  dispose(): void {
    this.disposed = true;
    this.unlisten();
    this.invalidateAll();
  }

  private invalidateAll(restarting = false): void {
    this.globalEpoch++;
    const retiring: Promise<void>[] = [];
    const entries = [...this.owners.values()].flatMap(owner => [...owner.values()]);
    for (const entry of entries) {
      this.invalidate(entry);
      if (restarting) retiring.push(entry.ready.catch(() => {}).then(() => this.close(entry)));
    }
    this.owners.clear();
    if (restarting) this.restartBarrier = Promise.allSettled([this.restartBarrier, ...retiring]).then(() => {});
  }

  private verifyLaunch(): void {
    // A request can race the supervisor's status notification after an exit.
    if (this.ready && !this.options.engine.isReady('job')) {
      this.ready = false;
      this.invalidateAll(true);
    }
  }

  private makeRoom(owner: number): void {
    const own = this.owners.get(owner);
    if (own && own.size >= MAX_PER_OWNER) {
      const idle = [...own.values()].filter(entry => entry.refs === 0).sort((a, b) => a.lastUsed - b.lastUsed)[0];
      if (!idle) throw new Error('Too many source sessions are active in this window.');
      this.invalidate(idle);
    }
    const all = [...this.owners.values()].flatMap(entries => [...entries.values()]);
    if (all.length >= MAX_TOTAL) {
      const idle = all.filter(entry => entry.refs === 0).sort((a, b) => a.lastUsed - b.lastUsed)[0];
      if (!idle) throw new Error('Too many source sessions are active.');
      this.invalidate(idle);
    }
  }

  private invalidate(entry: Entry): void {
    if (entry.invalid) return;
    entry.invalid = true;
    entry.abort.abort();
    if (entry.idle) clearTimeout(entry.idle);
    entry.idle = null;
    const entries = this.owners.get(entry.owner);
    if (entries?.get(entry.asset) === entry) {
      entries.delete(entry.asset);
      if (!entries.size) this.owners.delete(entry.owner);
    }
    if (entry.refs === 0) void entry.ready.catch(() => {}).then(() => this.close(entry)).catch(() => {});
  }

  private release(entry: Entry): void {
    entry.refs--;
    entry.lastUsed = Date.now();
    if (entry.refs !== 0) return;
    if (entry.invalid) { void this.close(entry).catch(() => {}); return; }
    entry.idle = setTimeout(() => this.invalidate(entry), IDLE_MS);
    entry.idle.unref?.();
  }

  private close(entry: Entry): Promise<void> {
    if (entry.closing) return entry.closing;
    const session = entry.session;
    if (session === null) return Promise.resolve();
    entry.session = null;
    entry.closing = (async () => {
      await this.options.engine.request('job', { type: 'closeSession' }, session).catch(() => {});
      await this.options.bindMediaSession?.(session, null);
    })();
    return entry.closing;
  }

  private async open(entry: Entry): Promise<number> {
    let session: number | null = null;
    try {
      let mark = performance.now();
      if (entry.invalid) throw new Error('The source session was invalidated.');
      await this.waitReady(entry.abort.signal);
      entry.phases.waitMs = performance.now() - mark;
      this.ready = true;
      if (entry.invalid) throw new Error('The source session was invalidated.');
      await this.restartBarrier;
      if (entry.invalid) throw new Error('The source session was invalidated.');
      mark = performance.now();
      const opened = await this.options.engine.request('job', { type: 'openSession', document: entry.document, role: 'jobSnapshot', process: 'job', color: STANDARD_COLOR });
      entry.phases.openMs = performance.now() - mark;
      if (opened.type !== 'sessionOpened') throw new Error('The job engine did not open a source session.');
      session = opened.session;
      entry.session = session;
      if (entry.invalid) throw new Error('The source session was invalidated.');
      mark = performance.now();
      await this.options.bindMediaSession?.(session, entry.project);
      entry.phases.bindMs = performance.now() - mark;
      if (entry.invalid) throw new Error('The source session was invalidated.');
      mark = performance.now();
      await this.resync(entry, entry.revision);
      entry.phases.resyncMs = performance.now() - mark;
      if (entry.invalid) throw new Error('The source session was invalidated.');
      return session;
    } catch (error) {
      if (session !== null) await this.close(entry).catch(() => {});
      throw error;
    }
  }

  private async advance(entry: Entry, revision: number): Promise<number> {
    let elapsed = 0;
    const next = entry.sync.then(async () => {
      if (entry.invalid) throw new Error('The source session was invalidated.');
      if (revision <= entry.revision) return;
      const mark = performance.now();
      await this.resync(entry, revision);
      if (entry.invalid) throw new Error('The source session was invalidated.');
      entry.revision = revision;
      elapsed = performance.now() - mark;
    });
    entry.sync = next;
    try { await next; } catch (error) { this.invalidate(entry); throw error; }
    return elapsed;
  }

  private async resync(entry: Entry, revision: number): Promise<void> {
    const session = entry.session;
    if (session === null) throw new Error('The source session was closed.');
    const source = entry.body.byteLength <= INLINE_BYTES ? { kind: 'inline' as const, body: entry.body }
      : { kind: 'cacheFile' as const, ...await this.options.engine.stageDocument('job', session, entry.body) };
    await this.options.engine.request('job', { type: 'resync', document: entry.document, revision,
      encoding: 'canonicalJsonV1', source, hash: entry.hash }, session);
  }

  private async waitReady(signal: AbortSignal): Promise<void> {
    if (this.options.engine.isReady('job')) return;
    await new Promise<void>((resolve, reject) => {
      const abort = () => finish(new Error('The source session was invalidated.'));
      const off = this.options.engine.listen({ status: () => {
        if (this.options.engine.isReady('job')) finish();
      } });
      const timer = setTimeout(() => finish(new Error('The job engine did not become ready.')), 45_000);
      const finish = (error?: Error) => {
        off(); clearTimeout(timer); signal.removeEventListener('abort', abort);
        if (error) reject(error); else resolve();
      };
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
      else if (this.options.engine.isReady('job')) finish();
    });
  }
}
