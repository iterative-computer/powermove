/*
 * Engine snapshots for windows (the Phase 3c render transport; plan
 * sections 18 to 20; docs/engine/rfcs/16-render-transport.md).
 *
 * A window asks main for a snapshot of its own engine session: through
 * `sendSync` when a thread waits for the pixels (`render.snapshot`,
 * `renderFrameTo`, and `raster` at apiVersion 1 to 3), through `invoke`
 * otherwise (the async siblings and the UI's thumbnails, library cards,
 * posters, and project thumbnail). Main names the window's port to the
 * interactive engine from the IPC sender (a window cannot name another's
 * session), sends the engine a host `snapshot` request with the window's
 * revision (read-your-writes), and hands back RGBA8 pixels: inline from
 * the answer, or mapped once out of the shared-memory segment the engine
 * wrote (pm-bridge `takePixels`), which Electron's IPC carries to the
 * renderer in shared memory again.
 *
 * Every reply is bounded: the request's own wait (with a Blender wait,
 * that too and the wait again after it), plus a render allowance, caps
 * how long main waits, and a window may have at most
 * `MAX_PER_WINDOW` snapshots outstanding and every window together at
 * most `MAX_OUTSTANDING_BYTES` of results. A reply is never an exception:
 * failures come back as `{ status: 'failed', code, message }`.
 *
 * Main never waits on the renderer here: the engine answers a snapshot
 * that needs pixels only the renderer draws with `needsRenderer`, which
 * goes straight back to the (possibly blocked) caller.
 */
import type { EngineSnapshotReply, EngineSnapshotRequest } from '../../shared/engine-ipc';
import type { Request, Response, SnapshotOutcome, SnapshotRequest } from '../../shared/engine-protocol';
import type { Log } from './connection';
import { RemoteError } from './protocol';
import type { WindowSnapshotSessions } from './window-sessions';

/** What the service needs of the engine (NativeEngine). */
export interface SnapshotEngine {
  isReady(process: 'interactive'): boolean;
  clientPort(id: number, process: 'interactive'): { port: number; generation: number } | null;
  request(process: 'interactive', body: Request, session?: number | null, revision?: number | null): Promise<Response>;
  /** The textures the engine sent ahead of a `needsRenderer` answer for
   *  one of its fallback requests (fallback-inputs.ts). */
  fallbackTextures?(key: string, count: number, width: number, height: number): Uint8Array[] | null;
}

/** Maps an engine's shared-memory segment once and unlinks it (pm-bridge). */
export type TakePixels = (name: string, length: number) => Uint8Array;

/** Longest a snapshot may ask the engine to wait (pm_protocol MAX_SNAPSHOT_WAIT_MS). */
export const MAX_WAIT_MS = 120_000;
/** Time main allows past the request's own wait for the render and its readback. */
export const RENDER_ALLOWANCE_MS = 10_000;
/** Longest a snapshot's Blender stage may run (pm_protocol MAX_BLENDER_WAIT_MS:
 *  today's 10-minute Blender timeout, plus the queue ahead of it). */
export const MAX_BLENDER_WAIT_MS = 900_000;

/** How long main waits for the engine's answer: the request's own wait,
 *  and with a Blender wait, the Blender stage and the wait again after it
 *  (the engine counts its wait afresh once Blender is done). */
export function answerBoundMs(snapshot: SnapshotRequest): number {
  const blender = snapshot.blenderWaitMs ?? 0;
  return (blender > 0 ? 2 * snapshot.waitMs + blender : snapshot.waitMs) + RENDER_ALLOWANCE_MS;
}
/** How long a snapshot waits for an interactive engine that is starting. */
export const READY_POLL_MS = 25;
/** Snapshots one window may have in flight. */
export const MAX_PER_WINDOW = 8;
/** Result bytes outstanding across every window. */
export const MAX_OUTSTANDING_BYTES = 1024 ** 3;
/** Largest layer object a raster may carry (pm_protocol MAX_LAYER_CONTENT_BYTES). */
export const MAX_LAYER_CONTENT_BYTES = 4 * 1024 * 1024;
/** Largest output edge (pm_protocol MAX_OUTPUT_DIMENSION). */
const MAX_EDGE = 16384;
const MAX_ID = 256;

const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const safeUint = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const id = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && new TextEncoder().encode(v).length <= MAX_ID;

/** Checks a window's snapshot request (IPC input: trust nothing). */
export function validSnapshotRequest(value: unknown): value is EngineSnapshotRequest {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  if (!safeUint(v['generation'])) return false;
  const document = v['document'];
  if (document !== undefined && !validDocument(document)) return false;
  if (v['session'] === null ? document === undefined : !safeUint(v['session'])) return false;
  const fallback = v['fallbackPixels'];
  if (fallback !== undefined && (document === undefined || !validFallback(fallback))) return false;
  const svg = v['svgPixels'];
  if (svg !== undefined && (document === undefined || !validSvgPixels(svg))) return false;
  if (v['revision'] !== null && !safeUint(v['revision'])) return false;
  const s = v['snapshot'] as Record<string, unknown> | null;
  if (!s || typeof s !== 'object') return false;
  if (!id(s['composition']) || !Number.isSafeInteger(s['time'])) return false;
  const size = s['outputSize'] as Record<string, unknown> | null;
  if (!size || !Number.isInteger(size['width']) || !Number.isInteger(size['height'])) return false;
  const w = size['width'] as number, h = size['height'] as number;
  if (w < 1 || h < 1 || w > MAX_EDGE || h > MAX_EDGE || w * h * 4 > 512 * 1024 ** 2) return false;
  if (!['opaque', 'premultiplied', 'straight'].includes(s['pixels'] as string)) return false;
  if (!['sync', 'async', 'idle'].includes(s['class'] as string)) return false;
  if (s['viewSeq'] !== null && !safeUint(s['viewSeq'])) return false;
  if (!Number.isInteger(s['waitMs']) || (s['waitMs'] as number) < 1 || (s['waitMs'] as number) > MAX_WAIT_MS) return false;
  // A Blender wait (today's `snapshotAsync` prepares the frame): never for
  // a blocked thread.
  const blender = s['blenderWaitMs'];
  if (blender !== undefined && (!Number.isInteger(blender) || (blender as number) < 0 || (blender as number) > MAX_BLENDER_WAIT_MS || ((blender as number) > 0 && s['class'] === 'sync'))) return false;
  const uploaded = s['uploaded'];
  if (!Array.isArray(uploaded) || uploaded.length > 64 || !uploaded.every((k) => typeof k === 'string' && k.length > 0 && k.length <= 512)) return false;
  const target = s['target'] as Record<string, unknown> | null;
  if (!target || typeof target !== 'object') return false;
  if (target['kind'] === 'frame') {
    return safeUint(target['motionBlurSamples']) && (target['motionBlurSamples'] as number) <= 256 &&
      typeof target['background'] === 'boolean' && typeof target['draft3d'] === 'boolean';
  }
  if (target['kind'] === 'layer') {
    const crop = target['crop'] as Record<string, unknown> | null;
    const cropOk = crop === null || (crop && typeof crop === 'object' && ['x', 'y', 'width', 'height'].every((k) => safeUint(crop[k])) && (crop['width'] as number) > 0 && (crop['height'] as number) > 0);
    const content = target['content'];
    const contentOk = content === undefined || content === null || (content instanceof Uint8Array && content.byteLength > 0 && content.byteLength <= MAX_LAYER_CONTENT_BYTES);
    return id(target['layer']) && finite(target['scale']) && (target['scale'] as number) >= 0.01 && (target['scale'] as number) <= 16 && !!cropOk && contentOk;
  }
  return false;
}

/** Largest document a window may send with a snapshot (canonical JSON). */
export const MAX_DOCUMENT_BYTES = 512 * 1024 * 1024;
/** Most fallback programs one request carries, and their bytes. */
export const MAX_FALLBACK_PIXELS = 16;
export const MAX_FALLBACK_BYTES = 256 * 1024 * 1024;
/** Most SVG rasters one request carries, and their bytes (a material
 *  map may be three.js's largest texture, 16384 × 16384). */
export const MAX_SVG_PIXELS = 64;
export const MAX_SVG_BYTES = 1024 * 1024 * 1024;
/** Largest SVG raster edge (`svgRasterDimensions`' limit). */
const MAX_SVG_EDGE = 8192;

function validDocument(value: unknown): boolean {
  if (!value || typeof value !== 'object') return false;
  const d = value as Record<string, unknown>;
  if (!id(d['id']) || !safeUint(d['revision'])) return false;
  const json = d['json'];
  if (json === undefined || json === null) return d['hash'] === undefined || d['hash'] === null || (typeof d['hash'] === 'string' && /^[0-9a-f]{64}$/.test(d['hash']));
  // A document's body comes with its canonical hash.
  return typeof d['hash'] === 'string' && /^[0-9a-f]{64}$/.test(d['hash']) && json instanceof Uint8Array && json.byteLength > 0 && json.byteLength <= MAX_DOCUMENT_BYTES;
}

function validSvgPixels(value: unknown): boolean {
  if (!Array.isArray(value) || value.length > MAX_SVG_PIXELS) return false;
  let bytes = 0;
  for (const p of value as Record<string, unknown>[]) {
    const key = p?.['key'] as Record<string, unknown> | undefined;
    if (!key || typeof key !== 'object' || typeof key['content'] !== 'string' || !/^[0-9a-f]{64}$/.test(key['content'])) return false;
    if (key['texture'] !== undefined && key['texture'] !== true) return false;
    const max = key['texture'] === true ? 16384 : MAX_SVG_EDGE;
    const w = key['width'], h = key['height'];
    if (!Number.isInteger(w) || !Number.isInteger(h) || (w as number) < 1 || (h as number) < 1 || (w as number) > max || (h as number) > max) return false;
    const data = p['data'];
    if (!(data instanceof Uint8Array) || data.byteLength !== (w as number) * (h as number) * 4) return false;
    bytes += data.byteLength;
  }
  return bytes <= MAX_SVG_BYTES;
}

function validFallback(value: unknown): boolean {
  if (!Array.isArray(value) || value.length > MAX_FALLBACK_PIXELS) return false;
  let bytes = 0;
  for (const p of value as Record<string, unknown>[]) {
    if (!p || typeof p !== 'object' || typeof p['key'] !== 'string' || p['key'].length === 0 || p['key'].length > 512) return false;
    if (!safeUint(p['width']) || !safeUint(p['height']) || (p['width'] as number) < 1 || (p['height'] as number) < 1) return false;
    const data = p['rgba16f'];
    if (!(data instanceof Uint8Array) || data.byteLength !== (p['width'] as number) * (p['height'] as number) * 8) return false;
    bytes += data.byteLength;
  }
  return bytes <= MAX_FALLBACK_BYTES;
}

const failed = (code: string, message: string): EngineSnapshotReply => ({ status: 'failed', code, message });

export type SnapshotServiceOptions = {
  engine: SnapshotEngine;
  /** Main's sessions for windows whose own session cannot serve a sync
   *  snapshot (window-sessions.ts). Absent: such requests fail. */
  windowSessions?: Pick<WindowSnapshotSessions, 'prepare'> | null;
  /** Absent when the pm-bridge addon did not load (then shared-memory results fail). */
  takePixels: TakePixels | null;
  log: Log;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
};

export type SnapshotStats = { served: number; failed: number; needsRenderer: number; shared: number; refused: number; outstanding: number; outstandingBytes: number; windowSessions: number; needsDocument: number };

export class SnapshotService {
  private readonly perWindow = new Map<number, number>();
  private outstandingBytes = 0;
  private readonly stats: SnapshotStats = { served: 0, failed: 0, needsRenderer: 0, shared: 0, refused: 0, outstanding: 0, outstandingBytes: 0, windowSessions: 0, needsDocument: 0 };

  constructor(private readonly options: SnapshotServiceOptions) {}

  status(): SnapshotStats {
    return { ...this.stats, outstanding: [...this.perWindow.values()].reduce((a, b) => a + b, 0), outstandingBytes: this.outstandingBytes };
  }

  /** A window's snapshot. Never rejects. */
  async snapshot(windowId: number, value: unknown): Promise<EngineSnapshotReply> {
    if (!validSnapshotRequest(value)) {
      this.stats.failed++;
      return failed('invalidMessage', 'A snapshot names its session, composition, time, size, and limits.');
    }
    const request = value;
    const bytes = request.snapshot.outputSize.width * request.snapshot.outputSize.height * 4;
    const inFlight = this.perWindow.get(windowId) ?? 0;
    if (inFlight >= MAX_PER_WINDOW || this.outstandingBytes + bytes > MAX_OUTSTANDING_BYTES) {
      this.stats.refused++;
      return failed('refused', 'Too many snapshots are in flight; try again when they finish.');
    }
    this.perWindow.set(windowId, inFlight + 1);
    this.outstandingBytes += bytes;
    try {
      return await this.run(windowId, request);
    } finally {
      const left = (this.perWindow.get(windowId) ?? 1) - 1;
      if (left > 0) this.perWindow.set(windowId, left);
      else this.perWindow.delete(windowId);
      this.outstandingBytes -= bytes;
    }
  }

  private async run(windowId: number, request: EngineSnapshotRequest): Promise<EngineSnapshotReply> {
    const now = this.options.now ?? (() => performance.now());
    const sleep = this.options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
    const started = now();
    const snapshot: SnapshotRequest = request.snapshot;
    const bound = answerBoundMs(snapshot);
    const deadline = started + bound;
    // A cold start or a restart: wait (within the request's own wait) for
    // the interactive engine rather than fail at once.
    while (!this.options.engine.isReady('interactive')) {
      if (now() - started >= snapshot.waitMs) {
        this.stats.failed++;
        return failed('unavailable', 'The new engine is not running.');
      }
      await sleep(READY_POLL_MS);
    }
    let owner: { port: number; session: number };
    if (request.document) {
      // The window's own session cannot serve it: main's session for the
      // window, at the document the window sent (window-sessions.ts).
      const sessions = this.options.windowSessions;
      if (!sessions) {
        this.stats.failed++;
        return failed('unavailable', 'This host has no snapshot sessions of its own.');
      }
      let prepared: Awaited<ReturnType<WindowSnapshotSessions['prepare']>> | 'timeout';
      let prepareTimer: ReturnType<typeof setTimeout> | null = null;
      try {
        // Bounded like the snapshot itself: a blocked window always gets a reply.
        prepared = await Promise.race([
          sessions.prepare(windowId, request.document, (request.fallbackPixels ?? []).map((p) => ({ ...p })), (request.svgPixels ?? []).map((p) => ({ key: { ...p.key }, data: p.data }))),
          new Promise<'timeout'>((resolve) => {
            prepareTimer = setTimeout(() => resolve('timeout'), Math.max(0, deadline - now()));
          })
        ]);
      } catch (error) {
        this.stats.failed++;
        if (error instanceof RemoteError) return failed(error.error.code, error.error.message);
        return failed('unavailable', error instanceof Error ? error.message : String(error));
      } finally {
        if (prepareTimer) clearTimeout(prepareTimer);
      }
      if (prepared === 'timeout') {
        this.stats.failed++;
        return failed('timeout', `Main's session for the window was not ready within ${bound} ms.`);
      }
      if (prepared === 'needsDocument') {
        this.stats.needsDocument++;
        return failed('needsDocument', 'Main has no copy of that document revision; send it with the request.');
      }
      owner = prepared;
      this.stats.windowSessions++;
    } else {
      const client = this.options.engine.clientPort(windowId, 'interactive');
      if (!client || client.generation !== request.generation || request.session === null) {
        this.stats.failed++;
        return failed('unknownSession', 'That engine session belongs to an earlier engine launch.');
      }
      owner = { port: client.port, session: request.session };
    }
    const answer = this.options.engine.request(
      'interactive',
      { type: 'snapshot', renderer: owner.port, session: owner.session, snapshot } as Request,
      null,
      request.revision
    );
    let timer: ReturnType<typeof setTimeout> | null = null;
    const timeout = new Promise<'timeout'>((resolve) => {
      timer = setTimeout(() => resolve('timeout'), Math.max(0, deadline - now()));
    });
    let response: Response | 'timeout';
    try {
      response = await Promise.race([answer, timeout]);
    } catch (error) {
      this.stats.failed++;
      if (error instanceof RemoteError) return failed(error.error.code, error.error.message);
      return failed('unavailable', error instanceof Error ? error.message : String(error));
    } finally {
      if (timer) clearTimeout(timer);
    }
    if (response === 'timeout') {
      this.stats.failed++;
      // A late answer still frees its segment at once.
      void answer.then((late) => this.discard(late), () => {});
      return failed('timeout', `The engine did not answer the snapshot within ${bound} ms.`);
    }
    if (response.type !== 'snapshot') {
      this.stats.failed++;
      return failed('invalidMessage', `The engine answered a snapshot with ${response.type}.`);
    }
    const outcome = response as unknown as SnapshotOutcome & { type: 'snapshot' };
    if (outcome.outcome === 'empty') {
      this.stats.served++;
      return { status: 'empty', revision: outcome.revision };
    }
    if (outcome.outcome === 'needsRenderer') {
      this.stats.needsRenderer++;
      // An effect's or a transition's textures arrived ahead of the answer
      // (the renderer may be blocked on it and cannot read its own port).
      const fallbackTextures: { key: string; textures: Uint8Array[] }[] = [];
      for (const r of outcome.fallback) {
        const count = r.textures ?? 0;
        if (count === 0) continue;
        const textures = this.options.engine.fallbackTextures?.(r.key, count, r.width, r.height) ?? null;
        if (textures) fallbackTextures.push({ key: r.key, textures });
      }
      return {
        status: 'needsRenderer', revision: outcome.revision, fallback: outcome.fallback, compat: outcome.compat, svg: outcome.svg ?? [],
        ...(fallbackTextures.length ? { fallbackTextures } : {})
      };
    }
    const expected = outcome.width * outcome.height * 4;
    let data: Uint8Array;
    try {
      data = this.pixels(outcome.payload, expected);
    } catch (error) {
      this.stats.failed++;
      return failed('internal', error instanceof Error ? error.message : String(error));
    }
    this.stats.served++;
    return {
      status: 'rendered',
      revision: outcome.revision,
      viewSeq: outcome.viewSeq,
      width: outcome.width,
      height: outcome.height,
      pixels: outcome.pixels,
      data,
      origin: [outcome.origin[0], outcome.origin[1]],
      layerBox: outcome.layerBox ? [outcome.layerBox[0], outcome.layerBox[1]] : null,
      raster: outcome.raster ?? null,
      quality: outcome.quality,
      waitedUs: outcome.waitedUs,
      renderUs: outcome.renderUs,
      mainMs: now() - started
    };
  }

  private pixels(payload: Extract<SnapshotOutcome, { outcome: 'rendered' }>['payload'], expected: number): Uint8Array {
    if (payload.kind === 'inline') {
      if (payload.bytes.byteLength !== expected) throw new Error(`the engine sent ${payload.bytes.byteLength} of ${expected} pixel bytes`);
      return payload.bytes;
    }
    if (payload.length !== expected) throw new Error(`the engine shared ${payload.length} of ${expected} pixel bytes`);
    if (!this.options.takePixels) throw new Error('main cannot read shared pixels without the frame bridge addon');
    this.stats.shared++;
    const data = this.options.takePixels(payload.name, payload.length);
    if (data.byteLength !== expected) throw new Error(`the shared segment held ${data.byteLength} of ${expected} bytes`);
    return data;
  }

  /** A late answer: take (and drop) its segment so it does not wait for the reaper. */
  private discard(response: Response): void {
    if (response.type !== 'snapshot') return;
    const outcome = response as unknown as SnapshotOutcome;
    if (outcome.outcome !== 'rendered' || outcome.payload.kind !== 'sharedMemory') return;
    try {
      this.options.takePixels?.(outcome.payload.name, outcome.payload.length);
    } catch (error) {
      this.options.log('warn', `a late snapshot's pixels could not be released: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}
