/* The Library card is a clone of the live viewer. Its #stage-inner keeps the
   live zoom and pan as inline geometry, and the copied #overlay is drawn in
   the same stage coordinates, so engine pixels must cover that box exactly:
   the full composition stretched over #stage-inner, never a refit. The live
   #gl may be a CSS crop of a zoomed viewport, so it is not the box to cover.
   Pure DOM helpers; the host's frozen-fork fill uses them too. */
import { previewRenderSize } from './preview-size';

/** The clone's composition box and its presentation canvas: only the known
 *  `#gl` directly inside `#stage-inner`, never another canvas. */
export function viewerCardTarget(clone: HTMLElement): { inner: HTMLElement; gl: HTMLCanvasElement } | null {
  const inner = clone.querySelector<HTMLElement>('[data-library-source-id="stage-inner"]');
  const gl = inner?.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-source-id="gl"]');
  return inner && gl ? { inner, gl } : null;
}

/** Render pixels for the clone's #stage-inner: what the live viewer renders
 *  for the full composition at the zoom it was cloned with (its CSS width over
 *  the composition width), its display density and preview quality, or the
 *  fixed preview resolution. Never above the composition: the cover's CSS
 *  stretch reproduces the live upscale. */
export function viewerCardSize(
  inner: HTMLElement, project: { w: number; h: number },
  density = 1, quality = 1, previewResolution?: string | number | null,
): { width: number; height: number } | null {
  const cssWidth = parseFloat(inner.style.width) || inner.offsetWidth;
  const cssHeight = parseFloat(inner.style.height) || inner.offsetHeight;
  const width = Number(project?.w), height = Number(project?.h);
  if (!(cssWidth >= 1 && cssHeight >= 1 && width >= 1 && height >= 1)) return null;
  // The live viewer's full-composition branch (viewer.ts layout).
  if (previewResolution && previewResolution !== 'auto') {
    return { width: Math.max(2, Math.round(width * quality)), height: Math.max(2, Math.round(height * quality)) };
  }
  const size = previewRenderSize(width, height, cssWidth / width, density, quality);
  return { width: size.width, height: size.height };
}

/** Covers the clone's #stage-inner with a full-composition frame. The
 *  stage-inner's own style and every sibling (the copied overlay, guides,
 *  source monitor) are left as cloned. A detached clone (a cached card off
 *  its shelf) paints too: canvases draw without layout, and the cover's CSS
 *  box is relative. */
export function paintViewerCard(clone: HTMLElement, frame: HTMLCanvasElement): boolean {
  const target = viewerCardTarget(clone);
  if (!target) return false;
  let cover = target.inner.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-engine-frame]');
  if (!cover) {
    cover = document.createElement('canvas');
    cover.dataset.libraryEngineFrame = '';
    target.gl.after(cover);
  }
  Object.assign(cover.style, { position: 'absolute', left: '0', top: '0', width: '100%', height: '100%', pointerEvents: 'none' });
  cover.width = frame.width;
  cover.height = frame.height;
  cover.getContext('2d')?.drawImage(frame, 0, 0);
  return true;
}
