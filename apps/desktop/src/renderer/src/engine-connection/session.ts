/*
 * A viewer session on the interactive engine that carries the document
 * (plan section 6, "The patch stream"; contract 5; the Phase 2 tracer
 * bullet).
 *
 * The session opens on the engine, takes the window's canonical document
 * as a resync (inline, or staged as a cache file when it is large), and
 * then follows pm-wasm's patch stream (`PM.EngineDoc.subscribe`): every
 * batch pm-wasm applied, in order. A batch the engine cannot place (a
 * revision gap, a body it refuses, another project) makes the session
 * resync; `load` batches (another project) resync too.
 *
 * Engine events for the session go to the viewer's presentation (frames,
 * surface pools, clock samples) and decide where the composition is drawn:
 * a session `unsupported` failure means the engine routed it to the legacy
 * viewer (capability routing, section 21); a frame means the engine draws
 * it.
 */
import type { ClockSample, Envelope, FrameDescriptor, PatchOrigin, ProjectColor, Request, SurfacePool, TransportCommand, ViewerState } from '../../../shared/engine-protocol';
import type { EngineConnection, Log } from './connection';

/** What the session needs from `PM.EngineDoc` (engine-doc/install.ts). */
export interface DocumentStream {
  readonly ready: boolean;
  whenReady: Promise<boolean>;
  subscribe(fn: (batch: StreamBatch) => void): () => void;
  snapshot(): { revision: number; encoding: 'canonicalJsonV1'; json: string; hash: string } | null;
}

export type StreamBatch = { baseRevision: number; revision: number; origin: PatchOrigin; count: number; body: Uint8Array };

/** Stages a large resync document with main (`nativeEngine.stageDocument`). */
export type StageDocument = (session: number, bytes: Uint8Array) => Promise<{ handle: string; length: number }>;

/** Where the session's composition is drawn. */
export type Route = 'pending' | 'engine' | 'legacy';

/** Messages for the viewer's presentation (engine-present/protocol.ts). */
export type ViewerMessage =
  | { type: 'frame'; frame: FrameDescriptor }
  | { type: 'surfacePool'; pool: SurfacePool }
  | { type: 'clock'; sample: ClockSample }
  | { type: 'routed'; route: 'legacy'; reason: string };

export type SessionHandlers = {
  message(message: ViewerMessage, arrivedMs: number): void;
  closed(reason: string): void;
};

export type SessionOptions = {
  connection: EngineConnection;
  stream: DocumentStream;
  /** The open project's id (the engine's document id). */
  documentId: () => string;
  stage?: StageDocument;
  handlers: SessionHandlers;
  log: Log;
  /** Route changes (the transport and the legacy compositor follow them). */
  onRoute?: (route: Route, reason: string | null) => void;
  /** Every clock sample of the session. */
  onClock?: (sample: ClockSample) => void;
  /** Resync bodies above this are staged as a cache file (bytes). */
  inlineLimit?: number;
};

export const DEFAULT_COLOR: ProjectColor = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' };

/** The largest batch body the protocol carries inline (pm_protocol MAX_PATCH_BODY_BYTES). */
export const MAX_PATCH_BODY_BYTES = 8 * 1024 * 1024;

/** Inline resyncs stay well under the 16 MiB message limit. */
export const INLINE_RESYNC_LIMIT = 12 * 1024 * 1024;

const encoder = new TextEncoder();

/** A hex digest as bytes. */
export function hexBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** A valid document id (contract: non-empty, at most 256 bytes). */
export function documentId(id: unknown): string {
  const text = typeof id === 'string' && id ? id : 'untitled';
  return encoder.encode(text).length <= 256 ? text : text.slice(0, 64);
}

export class DocumentSession {
  /** The engine's session id, once opened. */
  id: number | null = null;
  route: Route = 'pending';
  routeReason: string | null = null;
  /** The revision the engine has (acknowledged or resynced). */
  applied = 0;
  /** The latest clock sample. */
  clock: ClockSample | null = null;
  /** Frames received since the session opened, and the newest. */
  frames = 0;
  lastFrame: FrameDescriptor | null = null;
  resyncs = 0;
  batches = 0;
  failures: string[] = [];
  closed = false;
  private document = '';
  private pendingState: ViewerState | null = null;
  private unsubscribe: (() => void) | null = null;
  private unlisten: (() => void) | null = null;
  private offClose: (() => void) | null = null;
  /** Batches that wait for a resync in flight (a staged file resyncs asynchronously). */
  private queued: StreamBatch[] = [];
  private resyncing: Promise<void> | null = null;
  private resyncAgain = false;
  /** The revision the stream will base its next batch on. */
  private streamRevision = 0;

  constructor(private readonly options: SessionOptions) {}

  /** Opens the session, resyncs, and follows the stream. */
  async start(): Promise<void> {
    const { connection } = this.options;
    this.offClose = connection.onClose((reason) => this.end(reason));
    if (!(await this.options.stream.whenReady)) throw new Error('the document mirror is not available');
    if (this.closed) return;
    this.document = documentId(this.options.documentId());
    const opened = await connection.request({ type: 'openSession', document: this.document, role: 'interactive', process: 'interactive', color: DEFAULT_COLOR } as Request);
    if (opened.type !== 'sessionOpened') throw new Error(`openSession answered ${opened.type}`);
    if (this.closed) {
      void connection.request({ type: 'closeSession' } as Request, opened.session).catch(() => {});
      return;
    }
    this.id = opened.session;
    this.unlisten = connection.listen(opened.session, (message, arrived) => this.receive(message, arrived));
    this.unsubscribe = this.options.stream.subscribe((batch) => this.onBatch(batch));
    await this.resync('open');
    if (this.pendingState && !this.closed) {
      const state = this.pendingState;
      this.pendingState = null;
      this.sendViewerState(state);
    }
  }

  /** Replaces the engine's document with the current one. */
  resync(reason: string): Promise<void> {
    if (this.resyncing) {
      this.resyncAgain = true;
      return this.resyncing;
    }
    this.resyncing = this.doResync(reason).finally(() => {
      this.resyncing = null;
      if (this.resyncAgain && !this.closed) {
        this.resyncAgain = false;
        void this.resync('again');
      }
    });
    return this.resyncing;
  }

  private async doResync(reason: string): Promise<void> {
    const id = this.id;
    if (id === null || this.closed) return;
    const snapshot = this.options.stream.snapshot();
    if (!snapshot) throw new Error('the document mirror has no snapshot');
    this.document = documentId(this.options.documentId());
    const bytes = encoder.encode(snapshot.json);
    const limit = this.options.inlineLimit ?? INLINE_RESYNC_LIMIT;
    let source: { kind: 'inline'; body: Uint8Array } | { kind: 'cacheFile'; handle: string; length: number };
    if (bytes.length > limit && this.options.stage) {
      const staged = await this.options.stage(id, bytes);
      source = { kind: 'cacheFile', handle: staged.handle, length: staged.length };
    } else {
      source = { kind: 'inline', body: bytes };
    }
    // Batches after the snapshot follow it; earlier ones are in it.
    this.streamRevision = snapshot.revision;
    this.queued = this.queued.filter((b) => b.revision > snapshot.revision);
    const request = { type: 'resync', document: this.document, revision: snapshot.revision, encoding: 'canonicalJsonV1', source, hash: hexBytes(snapshot.hash) } as Request;
    try {
      await this.options.connection.request(request, id);
      this.applied = snapshot.revision;
      this.resyncs++;
    } catch (error) {
      if (this.closed) return;
      const text = error instanceof Error ? error.message : String(error);
      this.options.log('warn', `engine resync (${reason}) failed: ${text}`);
      this.failures.push(`resync: ${text}`);
      return;
    }
    // Batches that came while a staged resync was read follow it now.
    const queued = this.queued;
    this.queued = [];
    for (const batch of queued) this.forward(batch);
  }

  private onBatch(batch: StreamBatch): void {
    if (this.closed || this.id === null) return;
    if (batch.origin === 'load' || batch.body.length > MAX_PATCH_BODY_BYTES || batch.body.length === 0) {
      void this.resync(batch.origin === 'load' ? 'load' : 'large batch');
      return;
    }
    if (this.resyncing) {
      this.queued.push(batch);
      return;
    }
    this.forward(batch);
  }

  private forward(batch: StreamBatch): void {
    const id = this.id;
    if (id === null || this.closed) return;
    if (batch.revision <= this.streamRevision) return;
    if (batch.baseRevision !== this.streamRevision) {
      // A batch we never saw sits between: the engine needs the document.
      void this.resync('gap');
      return;
    }
    this.streamRevision = batch.revision;
    this.batches++;
    this.options.connection.event(
      { type: 'applyPatches', document: this.document, baseRevision: batch.baseRevision, revision: batch.revision, origin: batch.origin, encoding: 'canonicalV1', patchCount: Math.max(1, batch.count), body: batch.body },
      id
    );
  }

  private setRoute(route: Route, reason: string | null): void {
    if (route === this.route && reason === this.routeReason) return;
    this.route = route;
    this.routeReason = reason;
    this.options.onRoute?.(route, reason);
  }

  private receive(message: Envelope, arrivedMs: number): void {
    const body = message.message;
    if (body.kind !== 'event') return;
    const event = body.body as { type: string } & Record<string, unknown>;
    switch (event.type) {
      case 'frame': {
        const { type: _t, ...frame } = event;
        this.frames++;
        this.lastFrame = frame as unknown as FrameDescriptor;
        this.setRoute('engine', null);
        this.options.handlers.message({ type: 'frame', frame: this.lastFrame }, arrivedMs);
        return;
      }
      case 'surfacePool': {
        const { type: _t, ...pool } = event;
        this.options.handlers.message({ type: 'surfacePool', pool: pool as unknown as SurfacePool }, arrivedMs);
        return;
      }
      case 'clock': {
        const { type: _t, ...sample } = event;
        this.clock = sample as unknown as ClockSample;
        this.options.onClock?.(this.clock);
        this.options.handlers.message({ type: 'clock', sample: this.clock }, arrivedMs);
        return;
      }
      case 'patchesApplied':
        this.applied = Math.max(this.applied, Number(event['revision']) || 0);
        return;
      case 'failure': {
        const error = (event as unknown as { error: { code: string; message: string; retry: { kind: string } } }).error;
        if (error.code === 'unsupported') {
          this.setRoute('legacy', error.message);
          this.options.handlers.message({ type: 'routed', route: 'legacy', reason: error.message }, arrivedMs);
          return;
        }
        this.failures.push(`${error.code}: ${error.message}`);
        if (this.failures.length > 32) this.failures.shift();
        if (error.retry.kind === 'afterResync' || error.code === 'staleRevision') {
          void this.resync(error.code);
          return;
        }
        this.options.log(error.code === 'deviceLost' ? 'warn' : 'info', `engine: ${error.message}`);
        return;
      }
      default:
        return;
    }
  }

  sendViewerState(state: ViewerState): void {
    if (this.closed) return;
    if (this.id === null || this.resyncing && this.applied === 0) {
      this.pendingState = state;
      return;
    }
    this.options.connection.event({ type: 'viewer', ...state } as never, this.id, this.applied || null);
  }

  release(release: { poolIndex: number; seq: number }): void {
    if (this.id !== null && !this.closed) this.options.connection.event({ type: 'releaseFrame', ...release } as never, this.id);
  }

  transport(command: TransportCommand): void {
    if (this.id !== null && !this.closed) this.options.connection.event({ type: 'transport', ...command } as never, this.id);
  }

  /** Ends the session from this side. */
  close(): void {
    if (this.closed) return;
    const id = this.id;
    this.dispose();
    if (id !== null) void this.options.connection.request({ type: 'closeSession' } as Request, id).catch(() => {});
  }

  /** The connection went away (engine restart, page reload). */
  private end(reason: string): void {
    if (this.closed) return;
    this.dispose();
    this.options.handlers.closed(reason);
  }

  private dispose(): void {
    this.closed = true;
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.unlisten?.();
    this.unlisten = null;
    this.offClose?.();
    this.offClose = null;
    this.queued = [];
    this.setRoute('pending', null);
  }
}
