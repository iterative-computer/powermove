/*
 * The viewer's own state as engine view parameters (contract 2
 * `ViewerState`, plan section 18 "View override"): zoom and pan as the
 * viewer keeps them (CSS pixels per composition pixel; CSS pixels from the
 * centered position), the preview resolution menu, and the 3D editor
 * camera.
 *
 * The 3D editor has a Camera view (the composition's camera) and an Editor
 * view (a free camera the user orbits, pans, and dollies;
 * core/scene3d/viewport.ts). In the Editor view the override carries that
 * camera and the engine renders through it, as today's compositor does;
 * otherwise there is no override and the engine renders the composition
 * camera. The 3D editor has no viewport shading or X-ray (main removed the
 * Blender viewport, 2b1fa379a), so the override never sets them.
 */
import type { ViewParams } from './presentation';
import type { EditorCamera, PreviewResolution, ViewerCanvas } from './protocol';

export type ViewerLayoutState = {
  /** The zoom on screen (fit or chosen). */
  shown: number;
  fit: boolean;
  pan: readonly [number, number];
  /** `api.transport.previewResolution`: 'auto', a scale, or its text. */
  previewResolution: string | number | null | undefined;
  /** The 3D editor camera in the Editor view; null or absent in the Camera view. */
  editorCamera?: EditorCamera | null;
  playEveryFrame?: boolean;
  /** Today's viewer canvas for this view (`todaysCanvas`). */
  canvas?: ViewerCanvas | null;
};

/** The crop fields `todaysCanvas` reads (`PreviewViewport`). */
type CroppedViewport = { x: number; y: number; width: number; height: number };

/**
 * The canvas today's viewer draws a view into (decision V1): its backing
 * size `render` (`previewRenderSize`: `min(1, zoom × min(2, DPR)) ×
 * quality` of the composition below fit; the explicit preview resolution;
 * or the cropped viewport's `renderWidth` × `renderHeight`), the part of
 * the composition it holds (`viewport`, the crop of
 * `previewRenderViewport`; the whole composition when null), and its CSS
 * box in stage CSS pixels: the composition frame at `position` and `zoom`
 * (the canvas fills `V.inner`), or the crop inside it (`displayViewport`).
 * The browser scales the canvas into that box; the engine renders at the
 * canvas's size and scales it the same way. Null when it cannot be
 * described (the engine then draws at zoom × DPR).
 */
export function todaysCanvas(
  compWidth: number, compHeight: number,
  render: { width: number; height: number },
  viewport: CroppedViewport | null | undefined,
  position: { x: number; y: number }, zoom: number,
): ViewerCanvas | null {
  const crop = viewport
    ? { x: viewport.x, y: viewport.y, width: viewport.width, height: viewport.height }
    : { x: 0, y: 0, width: Number(compWidth), height: Number(compHeight) };
  const canvas: ViewerCanvas = {
    width: Math.round(Number(render.width)), height: Math.round(Number(render.height)), crop,
    css: { x: position.x + crop.x * zoom, y: position.y + crop.y * zoom, width: crop.width * zoom, height: crop.height * zoom },
  };
  const finite = [canvas.width, canvas.height, ...Object.values(crop), ...Object.values(canvas.css)].every(Number.isFinite);
  return finite && canvas.width >= 1 && canvas.height >= 1 && crop.width > 0 && crop.height > 0 && canvas.css.width > 0 && canvas.css.height > 0 ? canvas : null;
}

/** The preview resolution menu (Auto, Full, Half, Quarter) as contract 2's enum. */
export function previewResolution(value: string | number | null | undefined): PreviewResolution {
  const scale = typeof value === 'number' ? value : value == null || value === 'auto' ? NaN : Number(value);
  if (!Number.isFinite(scale)) return 'automatic';
  if (scale >= 0.75) return 'full';
  if (scale >= 0.375) return 'half';
  return 'quarter';
}

export function viewParamsFor(state: ViewerLayoutState): ViewParams {
  return {
    zoom: state.shown > 0 && Number.isFinite(state.shown) ? state.shown : 1,
    pan: state.fit ? [0, 0] : [state.pan[0], state.pan[1]],
    resolution: previewResolution(state.previewResolution),
    media: 'automatic',
    playEveryFrame: !!state.playEveryFrame,
    ...(state.canvas ? { canvas: state.canvas } : {}),
    ...(state.editorCamera ? { camera: state.editorCamera } : {})
  };
}
