/* Composition thumbnails for the Media panel.
 *
 * Each composition is rendered off-screen at a small size and kept as a JPEG
 * data URL. Rendering happens one comp per idle slice and never while the user
 * is interacting, navigating, playing or exporting, so a GPU readback cannot
 * stall a gesture. An existing thumbnail stays visible until its replacement
 * is ready, so edits never flash the placeholder icon. */
import { viewerService } from '../legacy/core/services';
import { prepareThumbnailVideoFrames, type ThumbnailVideoFrames } from './thumbnail-video-frames';

const WIDTH = 320;
const SETTLE_MS = 400;
const RETRY_MS = 500;
const EMPTY_VIDEO_FRAMES = new Map<string, never>();

type PMLike = Record<string, any>;

export interface CompThumbnails {
  readonly urls: Record<string, string>;
  /** Mark every composition stale; `key` changes whenever the project does. */
  refresh(key: string): void;
  dispose(): void;
}

/** The frame a composition is represented by: the middle of its work area. */
export function posterTime(comp: { dur?: number; work?: unknown }): number {
  const dur = Math.max(0, Number(comp.dur) || 0);
  const work = Array.isArray(comp.work) && comp.work.length === 2 ? comp.work.map(Number) : [0, dur];
  const start = Math.max(0, Math.min(dur, Number.isFinite(work[0]) ? work[0]! : 0));
  const end = Math.max(start, Math.min(dur, Number.isFinite(work[1]) ? work[1]! : dur));
  return (start + end) / 2;
}

/** Output size for a thumbnail, preserving the composition's aspect ratio. */
export function thumbnailSize(w: number, h: number): { width: number; height: number } {
  const cw = Math.max(1, Number(w) || 1), ch = Math.max(1, Number(h) || 1);
  const scale = Math.min(1, WIDTH / cw, WIDTH / ch);
  return { width: Math.max(2, Math.round(cw * scale)), height: Math.max(2, Math.round(ch * scale)) };
}

export function createCompThumbnails(PM: PMLike): CompThumbnails {
  const urls: Record<string, string> = $state({});
  const rendered = new Map<string, string>();
  let key = '';
  let timer = 0;
  let disposed = false;
  let canvas: HTMLCanvasElement | null = null;
  let capture: AbortController | null = null;
  let inFlight = false;
  let captureMonitor = 0;

  const blocked = () => PM.interactionActive?.() || viewerService(PM as any)?.isNavigating?.() || PM.playing
    || PM.agentFrameCapture || PM.Export?.busy || PM.Preview?.preparing || PM.Preview?.active;

  const schedule = (delay: number) => {
    if (disposed) return;
    window.clearTimeout(timer);
    timer = window.setTimeout(step, delay);
  };

  function render(id: string, renderedKey: string): string | null | Promise<string | null> {
    const project = PM.proj;
    const comp = project?.compId === id ? project : PM.Comps?.get?.(id);
    if (!comp || !Array.isArray(comp.layers) || !PM.GL?.renderToPixels) return null;
    const pending = prepareThumbnailVideoFrames(PM, comp, posterTime(comp), () => (capture ??= new AbortController()).signal);
    if (!pending) return encode(comp);
    return pending.then(prepared => {
      try {
        if (disposed || PM.proj !== project || key !== renderedKey || blocked()) return null;
        return encode(comp, prepared);
      } finally { prepared.dispose(); }
    });
  }

  function encode(comp: any, prepared?: ThumbnailVideoFrames): string | null {
    const project = PM.proj;
    const { width, height } = thumbnailSize(comp.w || project.w, comp.h || project.h);
    const quality = PM.quality;
    const oldFrames = PM.preparedVideoFrames, oldVersion = PM.preparedVideoVersion;
    const hadFrames = Object.hasOwn(PM, 'preparedVideoFrames'), hadVersion = Object.hasOwn(PM, 'preparedVideoVersion');
    let pixels: Uint8Array<ArrayBuffer> | null;
    try {
      PM.quality = 1;
      PM.preparedVideoFrames = prepared?.frames ?? EMPTY_VIDEO_FRAMES;
      PM.preparedVideoVersion = prepared?.version ?? 'thumbnail:empty';
      pixels = PM.GL.renderToPixels(posterTime(comp), width, height, {
        comp, mblur: false, mbSamples: 1, opaque: true, topDownOpaque: true,
        isolatedVideoFrames: true,
      });
    } finally {
      PM.quality = quality;
      if (hadFrames) PM.preparedVideoFrames = oldFrames; else delete PM.preparedVideoFrames;
      if (hadVersion) PM.preparedVideoVersion = oldVersion; else delete PM.preparedVideoVersion;
    }
    if (!pixels || pixels.length !== width * height * 4 || PM.GL.gl?.isContextLost?.()) return null;
    // Encoding finishes synchronously, so one bounded, detached canvas can
    // serve every composition without retaining a backing store per card.
    canvas ??= document.createElement('canvas');
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels.buffer, pixels.byteOffset, pixels.byteLength), width, height), 0, 0);
    return canvas.toDataURL('image/jpeg', .8);
  }

  function step() {
    timer = 0;
    if (disposed || !PM.proj || inFlight) return;
    if (blocked()) { schedule(RETRY_MS); return; }
    const ids: string[] = (PM.Comps?.list?.() ?? []).map((comp: { id: string }) => comp.id);
    const live = new Set(ids);
    for (const id of Object.keys(urls)) if (!live.has(id)) { delete urls[id]; rendered.delete(id); }
    const next = ids.find((id) => rendered.get(id) !== key);
    if (!next) return;
    const renderedKey = key;
    const project = PM.proj;
    let cancelledAttempt = false;
    const complete = (url: string | null) => {
      if (disposed || key !== renderedKey || PM.proj !== project || blocked()) return;
      if (url) urls[next] = url;
      // Failed captures retain the previous image, without spinning a retry loop.
      rendered.set(next, renderedKey);
    };
    const failed = (error: unknown) => {
      if ((error as any)?.name === 'AbortError') { cancelledAttempt = true; return; }
      if (!disposed && key === renderedKey && !blocked() && (error as any)?.name !== 'AbortError') console.warn('Composition thumbnail failed', error);
      complete(null);
    };
    try {
      const result = render(next, renderedKey);
      if (result instanceof Promise) {
        inFlight = true;
        // Only pending decodes need this guard. A gesture, playback or export
        // releases the private decoder promptly rather than competing with it.
        captureMonitor = window.setInterval(() => {
          if (blocked()) { capture?.abort(); capture = null; }
        }, 50);
        void result.then(complete, failed).finally(() => {
          inFlight = false;
          window.clearInterval(captureMonitor); captureMonitor = 0;
          schedule(key !== renderedKey || PM.proj !== project ? SETTLE_MS : blocked() || cancelledAttempt ? RETRY_MS : 0);
        });
        return;
      }
      complete(result);
    } catch (error) {
      failed(error);
    }
    schedule(0);
  }

  return {
    urls,
    refresh(next: string) {
      if (next !== key) { key = next; capture?.abort(); capture = null; }
      schedule(SETTLE_MS);
    },
    dispose() {
      disposed = true;
      window.clearTimeout(timer);
      window.clearInterval(captureMonitor); captureMonitor = 0;
      capture?.abort(); capture = null;
      if (canvas) { canvas.width = 0; canvas.height = 0; canvas = null; }
    },
  };
}
