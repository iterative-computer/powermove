/*
 * The viewer's own state as engine view parameters (contract 2
 * `ViewerState`, plan section 18 "View override"): zoom and pan as the
 * viewer keeps them (CSS pixels per composition pixel; CSS pixels from the
 * centered position), the preview resolution menu, and the 3D viewport's
 * shading and X-ray.
 *
 * In the editor view the override carries the editor camera and Local View
 * (`scene3d.viewport.state()`'s `engineCamera` and `localView`); in the
 * camera view it carries none and the engine renders the composition camera.
 */
import type { ViewParams } from './presentation';
import type { EditorCamera, PreviewResolution, Shading3d } from './protocol';

export type ViewerLayoutState = {
  /** The zoom on screen (fit or chosen). */
  shown: number;
  fit: boolean;
  pan: readonly [number, number];
  /** `api.transport.previewResolution`: 'auto', a scale, or its text. */
  previewResolution: string | number | null | undefined;
  /** The 3D viewport's state when the composition has a 3D scene. */
  scene3d?: { hasScene: boolean; shading: Shading3d; xray: boolean; camera?: EditorCamera | null; localView?: string[] | null } | null;
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
  const scene = state.scene3d?.hasScene ? state.scene3d : null;
  return {
    zoom: state.shown > 0 && Number.isFinite(state.shown) ? state.shown : 1,
    pan: state.fit ? [0, 0] : [state.pan[0], state.pan[1]],
    resolution: previewResolution(state.previewResolution),
    media: 'automatic',
    playEveryFrame: !!state.playEveryFrame,
    ...(scene ? { camera: scene.camera ?? null, shading: scene.shading, xray: scene.xray, ...(scene.localView?.length ? { localView: scene.localView } : {}) } : {})
  };
}
