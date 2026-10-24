/*
 * Main's snapshot sessions for windows (the Phase 3c render transport;
 * RFC 16, "Sync snapshots that cannot wait for the window's session").
 *
 * A synchronous snapshot blocks the window's renderer, so it can only be
 * served by the window's own engine session when that session already has
 * the window's document. Before the first session opens (boot, a closed
 * viewer), and while a staged resync is in flight (its next step needs the
 * blocked renderer), the window sends its document with the request
 * instead, and main serves the snapshot from a session it opens itself on
 * the interactive engine:
 *
 * - one session per window and engine launch, opened on main's host port
 *   (the engine lets the host snapshot sessions it owns);
 * - the window's definitions from main's relay (the same set the window
 *   pushes to its own sessions), pushed when they change;
 * - the document resynced when its revision or hash moves (inline up to
 *   `INLINE_LIMIT`, staged in a cache file above it); a request that names
 *   a revision main's session does not have and carries no document gets
 *   `needsDocument`, and the window sends it;
 * - shader fallback pixels and level 1 SVG rasters the window drew for a
 *   `needsRenderer` answer are posted to the session before the snapshot
 *   is asked again;
 * - media resolve through host services as for every session.
 *
 * A session closes when its window goes, after `IDLE_CLOSE_MS` unused, and
 * with its engine launch. Main never waits on the renderer here.
 */
import type { DefinitionsUpdate, Event, ProjectColor, Request, Response, SvgRasterKey } from '../../shared/engine-protocol';
import type { Log } from './connection';

/** A canonical hash's 32 bytes from its hex. */
function hexBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** Bodies at most this large resync inline; larger ones are staged. */
export const INLINE_LIMIT = 12 * 1024 * 1024;
/** A window's snapshot session closes after this long unused. */
export const IDLE_CLOSE_MS = 60_000;
/** Largest fallback band posted in one event (the renderer's band size). */
export const MAX_FALLBACK_BAND_BYTES = 8 * 1024 * 1024;
/** The project color every interactive session opens with (the renderer's
 *  `DEFAULT_COLOR`, engine-connection/session.ts). */
export const SESSION_COLOR: ProjectColor = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' };

export type WindowDocument = {
  /** The engine's document id. */
  id: string;
  revision: number;
  /** Main adopts this window's current native root without JSON or hashing. */
  nativeKernel?: true;
  /** The canonical hash (hex), with `json`. */
  hash?: string | null;
  /** Canonical JSON; absent when the window expects main to have it. */
  json?: Uint8Array | null;
};

export type FallbackPixels = { key: string; width: number; height: number; rgba16f: Uint8Array; rgba32f?: Uint8Array };
/** A level 1 SVG raster: premultiplied RGBA8, top-down (decision S1). */
export type SvgPixels = { key: SvgRasterKey; data: Uint8Array };
/** Largest SVG band posted in one event (the renderer's band size,
 *  `MAX_SVG_BAND_BYTES` in engine-connection/svg-rasters.ts). */
export const MAX_SVG_BAND_BYTES = 8 * 1024 * 1024;

export interface WindowSessionEngine {
  isReady(process: 'interactive'): boolean;
  /** Main's host port on the current interactive launch. */
  hostPort(process: 'interactive'): { port: number; generation: number } | null;
  request(process: 'interactive', body: Request, session?: number | null, revision?: number | null): Promise<Response>;
  stageDocument(process: 'interactive', session: number, bytes: Uint8Array): Promise<{ handle: string; length: number }>;
  event(process: 'interactive', body: Event, session: number | null): void;
  adoptNativeDocument?(owner: number, session: number, document: string, revision: number): Promise<void>;
}

export interface WindowDefinitions {
  snapshotFor(windowId: number, generation?: number): DefinitionsUpdate | null;
  /** The window's own definitions generation (changes on every update). */
  generationOf(windowId: number): number | null;
}

type Record = {
  launch: number;
  port: number;
  session: number;
  document: string;
  revision: number | null;
  hash: string | null;
  /** The window's definitions generation last pushed, and the stream's. */
  definitions: number | null;
  pushes: number;
  timer: ReturnType<typeof setTimeout> | null;
};

export type WindowSessionStats = { opened: number; resyncs: number; staged: number; definitions: number; closed: number; needsDocument: number };

export class WindowSnapshotSessions {
  private readonly windows = new Map<number, Record>();
  private readonly chains = new Map<number, Promise<unknown>>();
  readonly stats: WindowSessionStats = { opened: 0, resyncs: 0, staged: 0, definitions: 0, closed: 0, needsDocument: 0 };

  constructor(
    private readonly engine: WindowSessionEngine,
    private readonly definitions: WindowDefinitions,
    private readonly log: Log,
    private readonly idleCloseMs = IDLE_CLOSE_MS,
    /** Font-cache seeds for main-owned sessions still belong to the window. */
    private readonly sessionOwner?: (session: number, owner: number | null) => Promise<void>
  ) {}

  /** The window's session at `document`, ready for a snapshot: `{ port,
   *  session }`, or `needsDocument` when main lacks the revision and the
   *  request carried no document. Serialized per window. */
  prepare(windowId: number, document: WindowDocument, fallback: FallbackPixels[] = [], svg: SvgPixels[] = [], compat: readonly { [key: string]: unknown }[] = []): Promise<{ port: number; session: number } | 'needsDocument'> {
    const previous = this.chains.get(windowId) ?? Promise.resolve();
    const next = previous.catch(() => {}).then(() => this.run(windowId, document, fallback, svg, compat));
    this.chains.set(windowId, next);
    void next.finally(() => {
      if (this.chains.get(windowId) === next) this.chains.delete(windowId);
    }).catch(() => {});
    return next;
  }

  private async run(windowId: number, document: WindowDocument, fallback: FallbackPixels[], svg: SvgPixels[], compat: readonly { [key: string]: unknown }[] = []): Promise<{ port: number; session: number } | 'needsDocument'> {
    const host = this.engine.hostPort('interactive');
    if (!host) throw new Error('the new engine is not running');
    // Session ids may be reused after a restart. Clear every old binding
    // before opening any new session; a later stale close must not erase
    // another window's new font owner at the same id.
    for (const [id, record] of this.windows) {
      if (record.launch !== host.generation || record.port !== host.port) this.drop(id, false);
    }
    let record = this.windows.get(windowId);
    // A window's revisions only grow while its page lives (main forgets
    // the window's session when the page loads again), so the document id
    // and revision name the content.
    const sameDocument = record?.document === document.id && record.revision === document.revision;
    if (!sameDocument && !(document.nativeKernel || (document.json && document.hash))) {
      this.stats.needsDocument++;
      return 'needsDocument';
    }
    if (!record) {
      const opened = await this.engine.request('interactive', { type: 'openSession', document: document.id, role: 'interactive', process: 'interactive', color: SESSION_COLOR } as Request);
      if (opened.type !== 'sessionOpened') throw new Error(`openSession answered ${opened.type}`);
      record = { launch: host.generation, port: host.port, session: (opened as unknown as { session: number }).session, document: document.id, revision: null, hash: null, definitions: null, pushes: 0, timer: null };
      this.windows.set(windowId, record);
      this.stats.opened++;
      try {
        await this.sessionOwner?.(record.session, windowId);
      } catch (error) {
        // Never capture with a different window's optical-size cache seeds.
        this.drop(windowId, true);
        throw error;
      }
    }
    this.touch(windowId, record);
    // Definitions before the document, as a renderer session sends them
    // (contract 10, read-your-registrations).
    const generation = this.definitions.generationOf(windowId);
    if (generation !== null && generation !== record.definitions) {
      const update = this.definitions.snapshotFor(windowId, record.pushes + 1);
      if (update) {
        await this.engine.request('interactive', { type: 'pushDefinitions', ...update } as Request, record.session);
        record.pushes += 1;
        record.definitions = generation;
        this.stats.definitions++;
      }
    }
    if (!sameDocument) {
      if (document.nativeKernel) {
        if (!this.engine.adoptNativeDocument) throw new Error('The native document reference is unavailable');
        await this.engine.adoptNativeDocument(windowId, record.session, document.id, document.revision);
      } else {
        const json = document.json!;
        let source: { kind: 'inline'; body: Uint8Array } | { kind: 'cacheFile'; handle: string; length: number };
        if (json.byteLength > INLINE_LIMIT) {
          const staged = await this.engine.stageDocument('interactive', record.session, json);
          source = { kind: 'cacheFile', handle: staged.handle, length: staged.length };
          this.stats.staged++;
        } else {
          source = { kind: 'inline', body: json };
        }
        await this.engine.request('interactive', { type: 'resync', document: document.id, revision: document.revision, encoding: 'canonicalJsonV1', source, hash: hexBytes(document.hash!) } as unknown as Request, record.session);
      }
      record.document = document.id;
      record.revision = document.revision;
      record.hash = document.hash ?? null;
      this.stats.resyncs++;
    }
    for (const pixels of fallback) this.postFallback(record.session, pixels);
    for (const pixels of svg) this.postSvg(record.session, pixels);
    // The window's compatibility producers and answers (a `raster`
    // override): this session asks for, and draws, the window's layers as
    // the window's own session would.
    for (const event of compat) this.engine.event('interactive', event as unknown as Event, record.session);
    return { port: record.port, session: record.session };
  }

  /** Posts a level 1 SVG raster to the session in row bands, as the
   *  window posts them on its own session. */
  private postSvg(session: number, pixels: SvgPixels): void {
    const { key, data } = pixels;
    const rowBytes = key.width * 4;
    if (rowBytes <= 0 || data.byteLength !== rowBytes * key.height) {
      this.log('warn', `SVG raster ${key.width}x${key.height} pixels do not match their size; dropped`);
      return;
    }
    const rowsPerBand = Math.max(1, Math.floor(MAX_SVG_BAND_BYTES / rowBytes));
    for (let row = 0; row < key.height; row += rowsPerBand) {
      const rows = Math.min(rowsPerBand, key.height - row);
      this.engine.event('interactive', { type: 'compat', compat: 'svgPixels', key, row, rows, data: data.slice(row * rowBytes, (row + rows) * rowBytes) } as unknown as Event, session);
    }
  }

  /** Posts a fallback program's pixels to the session in bands. */
  private postFallback(session: number, pixels: FallbackPixels): void {
    const float32 = pixels.rgba32f !== undefined;
    const rowBytes = pixels.width * (float32 ? 16 : 8);
    const bytes = float32 ? pixels.rgba32f! : pixels.rgba16f;
    if (rowBytes <= 0 || bytes.byteLength !== rowBytes * pixels.height || (float32 && pixels.rgba16f.byteLength !== 0)) {
      this.log('warn', `fallback pixels for ${pixels.key.slice(0, 12)} do not match their size; dropped`);
      return;
    }
    const rowsPerBand = Math.max(1, Math.floor(MAX_FALLBACK_BAND_BYTES / rowBytes));
    for (let row = 0; row < pixels.height; row += rowsPerBand) {
      const rows = Math.min(rowsPerBand, pixels.height - row);
      const band = bytes.slice(row * rowBytes, (row + rows) * rowBytes);
      this.engine.event('interactive', { type: 'shaderFallback', fallback: 'pixels', key: pixels.key, width: pixels.width, height: pixels.height, row, rows, rgba16f: float32 ? new Uint8Array(0) : band, ...(float32 ? { rgba32f: band } : {}) } as unknown as Event, session);
    }
  }

  private touch(windowId: number, record: Record): void {
    if (record.timer) clearTimeout(record.timer);
    record.timer = setTimeout(() => {
      if (this.windows.get(windowId) === record) this.drop(windowId, true);
    }, this.idleCloseMs);
    (record.timer as { unref?: () => void }).unref?.();
  }

  /** Whether main holds a session for the window (status, tests). */
  has(windowId: number): boolean {
    return this.windows.has(windowId);
  }

  /** The window went (or no longer needs main's session). */
  forget(windowId: number): void {
    this.drop(windowId, true);
  }

  private drop(windowId: number, close: boolean): void {
    const record = this.windows.get(windowId);
    if (!record) return;
    this.windows.delete(windowId);
    if (record.timer) clearTimeout(record.timer);
    this.stats.closed++;
    void this.sessionOwner?.(record.session, null).catch((error: unknown) => {
      this.log('info', `forgetting snapshot font owner: ${error instanceof Error ? error.message : String(error)}`);
    });
    if (!close) return;
    const host = this.engine.hostPort('interactive');
    if (!host || host.generation !== record.launch) return;
    void this.engine.request('interactive', { type: 'closeSession' } as Request, record.session).catch((error: unknown) => {
      this.log('info', `closing window ${windowId}'s snapshot session: ${error instanceof Error ? error.message : String(error)}`);
    });
  }

  dispose(): void {
    for (const id of [...this.windows.keys()]) this.drop(id, true);
  }
}
