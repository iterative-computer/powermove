/* Composition thumbnails for the Media panel.
 *
 * Each composition is rendered off-screen at a small size and kept as a JPEG
 * data URL. Rendering happens one comp per idle slice and never while the user
 * is interacting, navigating, playing or exporting, so a GPU readback cannot
 * stall a gesture. An existing thumbnail stays visible until its replacement
 * is ready, so edits never flash the placeholder icon. */
import { viewerService } from '../legacy/core/services';

const WIDTH = 320;
const SETTLE_MS = 400;
const RETRY_MS = 500;

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

  const blocked = () => PM.interactionActive?.() || viewerService(PM as any)?.isNavigating?.() || PM.playing
    || PM.agentFrameCapture || PM.Export?.busy || PM.Preview?.preparing || PM.Preview?.active;

  const schedule = (delay: number) => {
    if (disposed) return;
    window.clearTimeout(timer);
    timer = window.setTimeout(step, delay);
  };

  function render(id: string): string | null {
    const project = PM.proj;
    const comp = project?.compId === id ? project : PM.Comps?.get?.(id);
    if (!comp || !Array.isArray(comp.layers) || !PM.GL?.renderToPixels) return null;
    const { width, height } = thumbnailSize(comp.w || project.w, comp.h || project.h);
    const quality = PM.quality;
    let pixels: Uint8Array<ArrayBuffer> | null;
    try {
      PM.quality = 1;
      pixels = PM.GL.renderToPixels(posterTime(comp), width, height, {
        comp, draft3d:true, mblur: false, mbSamples: 1, opaque: true, topDownOpaque: true,
      });
    } finally {
      PM.quality = quality;
    }
    if (!pixels || pixels.length !== width * height * 4 || PM.GL.gl?.isContextLost?.()) return null;
    const canvas = document.createElement('canvas');
    canvas.width = width; canvas.height = height;
    canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels.buffer, pixels.byteOffset, pixels.byteLength), width, height), 0, 0);
    return canvas.toDataURL('image/jpeg', .8);
  }

  function step() {
    timer = 0;
    if (disposed || !PM.proj) return;
    if (blocked()) { schedule(RETRY_MS); return; }
    const ids: string[] = (PM.Comps?.list?.() ?? []).map((comp: { id: string }) => comp.id);
    for (const id of Object.keys(urls)) if (!ids.includes(id)) { delete urls[id]; rendered.delete(id); }
    const next = ids.find((id) => rendered.get(id) !== key);
    if (!next) return;
    const renderedKey = key;
    try {
      const url = render(next);
      if (url) urls[next] = url;
    } catch (error) {
      console.warn('Composition thumbnail failed', error);
    }
    // Record the attempt either way so a comp that cannot render is not retried in a loop.
    rendered.set(next, renderedKey);
    schedule(0);
  }

  return {
    urls,
    refresh(next: string) {
      if (next !== key) key = next;
      schedule(SETTLE_MS);
    },
    dispose() {
      disposed = true;
      window.clearTimeout(timer);
    },
  };
}
