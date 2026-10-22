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
import type { EditorCamera, PreviewResolution } from './protocol';

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
};

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
    ...(state.editorCamera ? { camera: state.editorCamera } : {})
  };
}
