/*
 * The document host pool: one worker process per open project
 * (docs/engine/document-host.md, contract 15). The embedder (the serve
 * supervisor in engine/main.ts, the hidden MCP host) gives it host services
 * and calls `DocumentHostPoolApi`; the pool forks workers, routes calls by
 * project, answers each worker's service requests, fans out remote session
 * patches, and restarts a crashed worker with the desktop's backoff (1 s
 * doubling to 30 s, five tries), persisting state so a restart resumes the
 * same document and undo history.
 */
import { fork, type ChildProcess } from 'node:child_process';
import type { AgentToolRequestEvent, AgentToolResponseEvent } from '../../shared/ipc';
import type { ExtensionRecord } from '../../shared/extensions';
import type { PersistReason } from '../../shared/engine-protocol';
import {
  DOCUMENT_HOST_PROTOCOL, DOM_TOOLS, DocumentHostError, JOB_TOOLS,
  type DocumentHostFailure, type DocumentHostPoolApi, type DocumentHostServices, type ExtensionRegistration,
  type FromWorker, type InstanceOpened, type InstancePersisted, type InstanceStats, type OpenOptions, type ServiceName, type ToWorker,
} from './protocol';

/** A started worker: a forked process in production, an in-memory pair in tests. */
export interface WorkerHandle {
  readonly pid: number | null;
  send(message: ToWorker): void;
  onMessage(listener: (message: FromWorker) => void): void;
  onExit(listener: (code: number | null, signal: string | null) => void): void;
  kill(): void;
}

export interface PoolOptions {
  services: DocumentHostServices;
  /** "Use New Engine": workers require pm-wasm and the font service. */
  newEngine: boolean;
  /** Starts a worker. Default: forks `script` with `--document-worker`. */
  spawn?: () => WorkerHandle;
  /** The built engine bundle (engine.mjs) for the default spawn. */
  script?: string;
  /** Extra environment for workers (tests point POWERMOVE_PM_WASM here). */
  env?: Record<string, string | undefined>;
  /** How long a worker may take to start and open a project. */
  openTimeoutMs?: number;
  /** Restarts allowed after crashes before calls fail with `worker-crashed` (retry true). */
  maxRestarts?: number;
}

interface Instance {
  projectId: string;
  instance: number;
  handle: WorkerHandle;
  pending: Map<number, { resolve(value: unknown): void; reject(error: DocumentHostError): void }>;
  nextId: number;
  opened: Promise<InstanceOpened>;
  exited: boolean;
  closing: boolean;
}

const DEFAULT_OPEN_TIMEOUT = 60_000;
const DEFAULT_RESTARTS = 5;

export function forkWorker(script: string, env: Record<string, string | undefined>): WorkerHandle {
  const child: ChildProcess = fork(script, ['--document-worker'], {
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    serialization: 'advanced',
  });
  const exitListeners: Array<(code: number | null, signal: string | null) => void> = [];
  child.on('exit', (code, signal) => { for (const listener of exitListeners) listener(code, signal); });
  child.on('error', () => { /* reported through exit */ });
  // Drained so a chatty worker never blocks on a full pipe; its errors reach ours.
  child.stdout?.resume();
  child.stderr?.on('data', (chunk: Buffer) => process.stderr.write(chunk));
  return {
    get pid() { return child.pid ?? null; },
    send: (message) => { if (child.connected) child.send(message); },
    onMessage: (listener) => { child.on('message', (message) => listener(message as FromWorker)); },
    onExit: (listener) => { exitListeners.push(listener); },
    kill: () => { if (child.exitCode === null && child.signalCode === null) child.kill(); },
  };
}

export class DocumentHostPool implements DocumentHostPoolApi {
  readonly protocol = DOCUMENT_HOST_PROTOCOL;
  private readonly byProject = new Map<string, Instance>();
  private nextInstance = 1;
  private restarts = new Map<string, number>();
  private stopped = false;
  private offPatch: () => void;
  private offExtensions: () => void;

  constructor(private readonly options: PoolOptions) {
    this.offPatch = options.services.session.onPatch((patch) => {
      const instance = this.byProject.get(patch.projectId);
      if (instance && !instance.exited) instance.handle.send({ t: 'remote', patch });
    });
    // Every open project reloads what changed, as every editor window does.
    this.offExtensions = options.services.extensions?.onChanged((event) => {
      for (const instance of this.byProject.values()) if (!instance.exited) instance.handle.send({ t: 'extensionsChanged', event });
    }) ?? (() => {});
  }

  private log(line: string): void { this.options.services.log?.(line); }

  private spawn(): WorkerHandle {
    if (this.options.spawn) return this.options.spawn();
    if (!this.options.script) throw new DocumentHostError('worker-crashed', 'The document engine bundle (engine.mjs) is not built, so no document host worker can start. Run `bun run build` and restart the host.', false);
    return forkWorker(this.options.script, {
      POWERMOVE_DOCUMENT_HOST_NEW_ENGINE: this.options.newEngine ? '1' : '0',
      ...this.options.env,
    });
  }

  instances(): Array<{ projectId: string; instance: number; pid: number | null }> {
    return [...this.byProject.values()].map((i) => ({ projectId: i.projectId, instance: i.instance, pid: i.handle.pid }));
  }

  open(projectId: string, _options: OpenOptions = {}): Promise<InstanceOpened> {
    if (this.stopped) return Promise.reject(new DocumentHostError('closed', 'The document host is shutting down.'));
    const existing = this.byProject.get(projectId);
    if (existing && !existing.exited) return existing.opened;
    const handle = this.spawn();
    const instance: Instance = {
      projectId, instance: this.nextInstance++, handle, pending: new Map(), nextId: 1,
      opened: Promise.resolve(null as never), exited: false, closing: false,
    };
    this.byProject.set(projectId, instance);
    const ready = new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new DocumentHostError('worker-timeout', `The document host worker for ${projectId} did not start in time.`, true)), this.options.openTimeoutMs ?? DEFAULT_OPEN_TIMEOUT);
      timer.unref?.();
      handle.onMessage((message) => {
        if (message.t === 'ready') {
          clearTimeout(timer);
          if (message.protocol !== DOCUMENT_HOST_PROTOCOL) reject(new DocumentHostError('worker-crashed', `The document host worker speaks protocol ${message.protocol}, not ${DOCUMENT_HOST_PROTOCOL}.`));
          else resolve();
          return;
        }
        this.receive(instance, message);
      });
      handle.onExit((code, signal) => { clearTimeout(timer); this.exited(instance, code, signal); reject(new DocumentHostError('worker-crashed', `The document host worker for ${projectId} exited (${signal ?? code}) while starting.`, true)); });
    });
    instance.opened = ready.then(() => this.request<InstanceOpened>(instance, (id) => ({ t: 'open', id, projectId, instance: instance.instance, pinnedBy: _options.pinnedBy ?? null }), this.options.openTimeoutMs ?? DEFAULT_OPEN_TIMEOUT))
      .then((opened) => { this.restarts.delete(projectId); return opened; })
      .catch((error) => {
        // A worker that could not open its project is of no use: stop it, so
        // the next call starts again (named failures with retry say so).
        if (this.byProject.get(projectId) === instance) this.byProject.delete(projectId);
        instance.closing = true;
        handle.kill();
        throw error instanceof DocumentHostError ? error : new DocumentHostError('worker-error', String((error as Error)?.message ?? error));
      });
    return instance.opened;
  }

  private request<T>(instance: Instance, build: (id: number) => ToWorker, timeoutMs?: number): Promise<T> {
    if (instance.exited) return Promise.reject(new DocumentHostError('worker-crashed', `The document host worker for ${instance.projectId} is not running.`, true));
    const id = instance.nextId++;
    return new Promise<T>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | null = null;
      if (timeoutMs) {
        timer = setTimeout(() => { instance.pending.delete(id); reject(new DocumentHostError('worker-timeout', `The document host worker for ${instance.projectId} did not answer in time.`, true)); }, timeoutMs);
        timer.unref?.();
      }
      instance.pending.set(id, {
        resolve: (value) => { if (timer) clearTimeout(timer); resolve(value as T); },
        reject: (error) => { if (timer) clearTimeout(timer); reject(error); },
      });
      instance.handle.send(build(id));
    });
  }

  private receive(instance: Instance, message: FromWorker): void {
    switch (message.t) {
      case 'reply': {
        const pending = instance.pending.get(message.id);
        if (!pending) return;
        instance.pending.delete(message.id);
        if (message.ok) pending.resolve(message.value);
        else pending.reject(DocumentHostError.from(message.failure as DocumentHostFailure));
        return;
      }
      case 'service':
        void this.serve(message.name, message.args).then(
          (value) => instance.handle.send({ t: 'serviceReply', id: message.id, ok: true, value }),
          (error) => instance.handle.send({ t: 'serviceReply', id: message.id, ok: false, error: error instanceof Error ? error.message : String(error) }),
        );
        return;
      case 'log':
        this.log(message.line);
        return;
    }
  }

  private progressWired = new WeakSet<object>();

  private async serve(name: ServiceName, args: unknown[]): Promise<unknown> {
    const s = this.options.services;
    const missing = (what: string) => { throw new DocumentHostError('service-unavailable', `${what} is not available on this host.`); };
    switch (name) {
      case 'store.read': return s.store.read(String(args[0]));
      case 'store.write': return s.store.write(String(args[0]), args[1]);
      case 'store.remove': return s.store.remove(String(args[0]));
      case 'session.join': return s.session.join(String(args[0]));
      case 'session.patch': return s.session.patch(String(args[0]), args[1] as never);
      case 'session.leave': s.session.leave(String(args[0])); return null;
      case 'mediaPath.lookup': return s.mediaPath ? s.mediaPath.lookup(args[0] as never) : missing('Project media lookup');
      case 'transcription.status':
        // No transcription service: captions say so, as the web host does.
        if (!s.transcription) return { models: [], activeModelId: null, available: false };
        this.wireProgress();
        return s.transcription.status();
      case 'transcription.transcribe': return s.transcription ? s.transcription.transcribe(args[0] as never) : missing('Transcription');
      case 'transcription.cancelTranscribe': return s.transcription ? s.transcription.cancelTranscribe(String(args[0])) : null;
      // No extensions service: the built-ins only (an empty list, as a bridge-less editor boots).
      case 'extensions.list': return s.extensions ? s.extensions.list() : null;
      case 'extensions.bundle': return s.extensions ? s.extensions.bundle(String(args[0])) : missing('Extension bundles');
      case 'extensions.values': return s.extensions?.values ? s.extensions.values(String(args[0])) : {};
    }
  }

  private wireProgress(): void {
    const t = this.options.services.transcription;
    if (!t || this.progressWired.has(t)) return;
    this.progressWired.add(t);
    t.onProgress((progress) => { for (const i of this.byProject.values()) if (!i.exited) i.handle.send({ t: 'progress', progress }); });
  }

  private exited(instance: Instance, code: number | null, signal: string | null): void {
    if (instance.exited) return;
    instance.exited = true;
    for (const [, pending] of instance.pending) pending.reject(new DocumentHostError('worker-crashed', `The document host worker for ${instance.projectId} exited (${signal ?? code}).`, true));
    instance.pending.clear();
    if (this.byProject.get(instance.projectId) === instance) this.byProject.delete(instance.projectId);
    if (instance.closing || this.stopped) return;
    const count = (this.restarts.get(instance.projectId) ?? 0) + 1;
    this.restarts.set(instance.projectId, count);
    this.log(`[engine] document host worker for ${instance.projectId} exited (${signal ?? code}); attempt ${count}`);
  }

  /** The open instance for a call, restarting a crashed one within the backoff budget. */
  private async ready(projectId: string): Promise<Instance> {
    const restarts = this.restarts.get(projectId) ?? 0;
    if (!this.byProject.has(projectId) && restarts > 0) {
      if (restarts > (this.options.maxRestarts ?? DEFAULT_RESTARTS)) {
        throw new DocumentHostError('worker-crashed', `The document host worker for ${projectId} keeps exiting; the host log has the reason.`, true);
      }
      const delay = Math.min(30_000, 1000 * 2 ** (restarts - 1));
      await new Promise((resolve) => setTimeout(resolve, delay).unref?.());
    }
    await this.open(projectId);
    return this.byProject.get(projectId)!;
  }

  async call(projectId: string, request: AgentToolRequestEvent): Promise<AgentToolResponseEvent> {
    if ((DOM_TOOLS as readonly string[]).includes(request.tool) || (JOB_TOOLS as readonly string[]).includes(request.tool)) {
      throw new DocumentHostError('wrong-route', `${request.tool} does not run in a document host (contract 15 routes it elsewhere).`);
    }
    const instance = await this.ready(projectId);
    return this.request<AgentToolResponseEvent>(instance, (id) => ({ t: 'tool', id, request }));
  }

  async persist(projectId: string, reason: PersistReason): Promise<InstancePersisted> {
    const instance = this.byProject.get(projectId);
    if (!instance) throw new DocumentHostError('closed', `No document host instance is open for ${projectId}.`);
    await instance.opened;
    return this.request<InstancePersisted>(instance, (id) => ({ t: 'persist', id, reason }));
  }

  async registerExtension(projectId: string, record: ExtensionRecord): Promise<ExtensionRegistration> {
    const instance = await this.ready(projectId);
    return this.request<ExtensionRegistration>(instance, (id) => ({ t: 'register', id, record }));
  }

  async stats(projectId: string): Promise<InstanceStats> {
    const instance = this.byProject.get(projectId);
    if (!instance) throw new DocumentHostError('closed', `No document host instance is open for ${projectId}.`);
    await instance.opened;
    return this.request<InstanceStats>(instance, (id) => ({ t: 'stats', id }));
  }

  async close(projectId: string): Promise<void> {
    const instance = this.byProject.get(projectId);
    if (!instance) return;
    instance.closing = true;
    try {
      await instance.opened;
      await this.request(instance, (id) => ({ t: 'close', id }), 30_000);
    } catch (error) {
      this.log(`[engine] closing ${projectId}: ${(error as Error).message}`);
    } finally {
      if (this.byProject.get(projectId) === instance) this.byProject.delete(projectId);
      await this.stop(instance);
    }
  }

  private stop(instance: Instance): Promise<void> {
    if (instance.exited) return Promise.resolve();
    return new Promise<void>((resolve) => {
      instance.handle.onExit(() => resolve());
      instance.handle.kill();
      setTimeout(resolve, 5_000).unref?.();
    });
  }

  async shutdown(): Promise<void> {
    this.stopped = true;
    this.offPatch();
    this.offExtensions();
    // Persist every instance first (contract 15 invariant 5), then stop them.
    await Promise.all([...this.byProject.keys()].map((projectId) => this.close(projectId)));
  }
}
