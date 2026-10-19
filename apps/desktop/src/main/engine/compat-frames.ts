/*
 * Compatibility pixels for exports (the Phase 3c render transport; plan
 * section 20, the `services` `raster` override and the compatibility
 * WebGL2 context; contract 14 `compatFrame`).
 *
 * Some layers can only be drawn by the document's renderer: a `raster`
 * override's result, an extension's offscreen WebGL2 compatibility
 * context. The interactive engine gets them as the renderer uploads them;
 * the job engine, rendering an export frame nobody previewed, asks the
 * host. Main asks every editor window; the one with the document open
 * draws the layer at that frame and answers with its pixels, the others
 * answer at once that they do not have it. Main hands the first pixels to
 * the job engine inline or through a shared-memory segment the engine
 * takes once.
 *
 * Bounded everywhere: one answer per ask, `ASK_TIMEOUT_MS` per ask,
 * `MAX_ASKS` in flight, and a segment the engine never takes is unlinked
 * after `SEGMENT_ORPHAN_MS`.
 */
import type { CompatFrameRequest, Response } from '../../shared/engine-protocol';
import type { EngineCompatAnswer, EngineCompatAsk } from '../../shared/engine-ipc';
import { ProtocolError } from './protocol';

/** Largest answer inline (pm_protocol INLINE_PIXEL_BYTES). */
export const INLINE_PIXEL_BYTES = 1024 * 1024;
export const ASK_TIMEOUT_MS = 30_000;
export const MAX_ASKS = 32;
export const SEGMENT_ORPHAN_MS = 60_000;
const MAX_EDGE = 16384;

/** An editor window that may draw compatibility pixels. */
export interface CompatWindow {
  id: number;
  /** Posts an ask to the window's renderer. */
  ask(ask: EngineCompatAsk): void;
}

export type CompatFramesOptions = {
  windows: () => CompatWindow[];
  /** pm-bridge's shared-memory writer and its cleanup (absent without the addon). */
  publishPixels: ((bytes: Uint8Array) => string) | null;
  unlinkPixels: ((name: string) => void) | null;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

type Pending = { windows: Set<number>; last: string; resolve: (answer: EngineCompatAnswer) => void; timer: { cancel(): void } };

const defaultSchedule = (fn: () => void, ms: number) => {
  const t = setTimeout(fn, ms);
  return { cancel: () => clearTimeout(t) };
};

export function validCompatAnswer(value: unknown): value is EngineCompatAnswer {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  if (!Number.isSafeInteger(v['ask'])) return false;
  if (v['status'] === 'unavailable') return typeof v['message'] === 'string';
  if (v['status'] !== 'pixels') return false;
  const w = v['width'], h = v['height'];
  if (!Number.isInteger(w) || !Number.isInteger(h) || (w as number) < 1 || (h as number) < 1 || (w as number) > MAX_EDGE || (h as number) > MAX_EDGE) return false;
  const bpp = v['format'] === 'rgba16fPremultiplied' ? 8 : v['format'] === 'rgba8Premultiplied' || v['format'] === 'rgba8Straight' ? 4 : 0;
  if (!bpp || !(v['data'] instanceof Uint8Array) || v['data'].byteLength !== (w as number) * (h as number) * bpp) return false;
  const origin = v['origin'];
  return typeof v['key'] === 'string' && v['key'].length > 0 && v['key'].length <= 512 &&
    Array.isArray(origin) && origin.length === 2 && origin.every((n) => Number.isFinite(n)) &&
    typeof v['scale'] === 'number' && Number.isFinite(v['scale']) && v['scale'] > 0 && v['scale'] <= 16;
}

export class CompatFrames {
  private next = 1;
  private readonly pending = new Map<number, Pending>();

  constructor(private readonly options: CompatFramesOptions) {}

  /** The job engine's `compatFrame` request. */
  async serve(request: CompatFrameRequest): Promise<Response> {
    if (this.pending.size >= MAX_ASKS) throw new ProtocolError('budgetExceeded', 'Too many compatibility frames are being drawn.');
    const windows = this.options.windows();
    // Level 1 SVG images are Chromium's rasters, drawn by the window that
    // has the project open (decision S1).
    const closed = request.source === 'svgImage'
      ? 'SVG images are drawn by the editor window that has this project open; open the project in a window to export it.'
      : 'This layer is drawn by an extension in the editor; open the project in a window to export it.';
    if (!windows.length) throw new ProtocolError('notHeadlessRenderable', closed);
    const ask = this.next++;
    const answer = await new Promise<EngineCompatAnswer>((resolve) => {
      const timer = (this.options.schedule ?? defaultSchedule)(() => {
        this.pending.delete(ask);
        resolve({ ask, status: 'unavailable', message: `The editor did not draw the layer within ${ASK_TIMEOUT_MS / 1000} s.` });
      }, ASK_TIMEOUT_MS);
      this.pending.set(ask, { windows: new Set(windows.map((w) => w.id)), last: closed, resolve, timer });
      for (const window of windows) window.ask({ ask, request });
    });
    if (answer.status !== 'pixels') throw new ProtocolError('notHeadlessRenderable', answer.message.slice(0, 1024));
    let payload: { kind: 'inline'; bytes: Uint8Array } | { kind: 'sharedMemory'; name: string; length: number };
    if (answer.data.byteLength <= INLINE_PIXEL_BYTES) {
      payload = { kind: 'inline', bytes: answer.data };
    } else {
      if (!this.options.publishPixels) throw new ProtocolError('internal', 'Main cannot share pixels without the frame bridge addon.');
      const name = this.options.publishPixels(answer.data);
      // Taken by the engine at once; unlinked here if it never is.
      (this.options.schedule ?? defaultSchedule)(() => this.options.unlinkPixels?.(name), SEGMENT_ORPHAN_MS);
      payload = { kind: 'sharedMemory', name, length: answer.data.byteLength };
    }
    return { type: 'compatFrame', key: answer.key, width: answer.width, height: answer.height, format: answer.format, origin: answer.origin, scale: answer.scale, payload } as Response;
  }

  /** A window's answer (IPC input). Only an asked window may answer, once:
   *  the first pixels win; when every asked window said no, the last
   *  reason is the answer. */
  answer(windowId: number, value: unknown): boolean {
    if (!validCompatAnswer(value)) return false;
    const pending = this.pending.get(value.ask);
    if (!pending || !pending.windows.delete(windowId)) return false;
    if (value.status === 'unavailable') {
      pending.last = value.message;
      if (pending.windows.size) return true;
    }
    this.pending.delete(value.ask);
    pending.timer.cancel();
    pending.resolve(value.status === 'pixels' ? value : { ask: value.ask, status: 'unavailable', message: pending.last });
    return true;
  }

  /** A window closed: it no longer answers its asks. */
  windowGone(windowId: number): void {
    for (const [ask, pending] of [...this.pending]) {
      if (!pending.windows.delete(windowId) || pending.windows.size) continue;
      this.pending.delete(ask);
      pending.timer.cancel();
      pending.resolve({ ask, status: 'unavailable', message: pending.last });
    }
  }
}
