/*
 * The viewer's own state as engine view parameters (contract 2
 * `ViewerState`, plan section 18 "View override"): zoom and pan as the
 * viewer keeps them (CSS pixels per composition pixel; CSS pixels from the
 * centered position), the preview resolution menu, and the 3D viewport's
 * shading and X-ray.
 *
 * The editor camera is not in the 3D viewport's API yet (it lives in the
 * three.js viewport); until the 3D lane exposes its view and projection,
 * the override carries shading and X-ray with no camera, and the engine
 * renders the composition camera.
 */
import type { ViewParams } from './presentation';
import type { PreviewResolution, Shading3d } from './protocol';

export type ViewerLayoutState = {
  /** The zoom on screen (fit or chosen). */
  shown: number;
  fit: boolean;
  pan: readonly [number, number];
  /** `api.transport.previewResolution`: 'auto', a scale, or its text. */
  previewResolution: string | number | null | undefined;
  /** The 3D viewport's state when the composition has a 3D scene. */
  scene3d?: { hasScene: boolean; shading: Shading3d; xray: boolean } | null;
  playEveryFrame?: boolean;
  /** Today's output scale for this view (`todaysOutputScale`). */
  svgScale?: readonly [number, number] | null;
};

/** The viewport fields `todaysOutputScale` reads (`PreviewViewport`). */
type CroppedViewport = { width: number; height: number };

/**
 * Today's compositor's output scale (`outputScale(W, H)` in
 * legacy/gl/compositor.ts) on the canvas today's viewer sizes for a view:
 * canvas pixels per composition pixel, per axis. `render` is that canvas
 * (`previewRenderSize`, the explicit preview resolution, or the cropped
 * viewport's `renderWidth` × `renderHeight`), `viewport` the crop
 * (`previewRenderViewport`), null when the whole composition is drawn.
 * Level 1 SVG images pick their bucket at this scale
 * (`svgRasterDimensions(…, scaledWorld(L, T, W, H))`), and today's viewer
 * caps it at one pixel per composition pixel below fit (`min(1, zoom ×
 * DPR) × quality`), where the engine draws at zoom × DPR. Null when it
 * cannot be computed (the engine then keeps its own scale).
 */
export function todaysOutputScale(
  compWidth: number, compHeight: number,
  render: { width: number; height: number },
  viewport: CroppedViewport | null | undefined,
): [number, number] | null {
  const W = Number(render.width), H = Number(render.height);
  const scale: [number, number] = viewport
    ? [W / Math.max(1e-6, viewport.width), H / Math.max(1e-6, viewport.height)]
    : [W / Math.max(1, Number(compWidth) || W), H / Math.max(1, Number(compHeight) || H)];
  return scale.every((v) => Number.isFinite(v) && v > 0) ? scale : null;
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
  const scene = state.scene3d?.hasScene ? state.scene3d : null;
  return {
    zoom: state.shown > 0 && Number.isFinite(state.shown) ? state.shown : 1,
    pan: state.fit ? [0, 0] : [state.pan[0], state.pan[1]],
    resolution: previewResolution(state.previewResolution),
    media: 'automatic',
    playEveryFrame: !!state.playEveryFrame,
    ...(state.svgScale ? { svgScale: [state.svgScale[0], state.svgScale[1]] as [number, number] } : {}),
    ...(scene ? { camera: null, shading: scene.shading, xray: scene.xray } : {})
  };
}
