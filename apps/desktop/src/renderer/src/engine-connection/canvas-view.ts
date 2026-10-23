import type { PMRegistry } from '../legacy/registry';
import type { ViewParams } from '../../../extensions/viewer/engine-present/presentation';
import { viewParamsFor } from '../../../extensions/viewer/engine-present/view-params';
import { hasScene3D } from '../core/scene3d/controller';
import { engineCamera } from '../core/scene3d/commands';
import { editModeLayer } from '../core/scene3d/editmode';
import { activeObject, onViewportSettings, selected3D, viewportPreferences } from '../core/scene3d/editor-state';
import { onRenderState, renderState } from '../core/scene3d/rendering';
import { localViewIds, onViewportChange } from '../core/scene3d/viewport';

/** Editor-only state for a pinned canvas or card preview. The public viewport
 * snapshot also asks for the render camera; this one never does. */
export function engineCanvasSceneState(PM: PMRegistry) {
  if (!hasScene3D(PM)) return null;
  const prefs = viewportPreferences(PM);
  const editMode = editModeLayer(PM);
  const local = localViewIds(PM);
  return {
    hasScene: true, shading: renderState(PM).shading, xray: prefs.xray, overlays: prefs.overlays,
    selected: editMode ? [] : selected3D(PM), active: editMode ? null : activeObject(PM),
    editMode: !!editMode, engineCamera: engineCamera(PM), localView: local ? [...local] : null
  };
}

/** A bound viewer reads no native render-camera facts during startup. */
export function engineCanvasViewParams(PM: PMRegistry): ViewParams {
  const layout = { shown: 1, fit: true, pan: [0, 0] as const, previewResolution: 'auto' };
  if (!PM.EngineDoc?.ready) return viewParamsFor(layout);
  const scene = engineCanvasSceneState(PM);
  return viewParamsFor({ ...layout, scene3d: scene ? { ...scene, camera: scene.engineCamera } : null });
}

/** The same editor notifications as viewport.onChange, without its eager
 * full viewportSnapshot (and therefore without a native render-camera ask). */
export function watchEngineCanvasView(PM: PMRegistry, refresh: () => void): () => void {
  let live = true, queued = false;
  const fire = (): void => {
    if (queued || !live) return;
    queued = true;
    queueMicrotask(() => { queued = false; if (live) refresh(); });
  };
  const offs = [onViewportChange(PM, fire), onViewportSettings(PM, fire), onRenderState(PM, fire),
    ...['sel', 'selection', 'scene3d:selection', 'scene3d:viewport', 'project', 'composition', 'layers'].map(event => PM.bus?.on?.(event, fire))];
  const doc = PM.EngineDoc;
  const offReady = doc?.onReady?.(fire);
  if (doc?.enabled && !doc.ready) void doc.whenReady.then((ok: boolean) => { if (ok && doc.ready && PM.EngineDoc === doc) fire(); });
  return () => { live = false; offReady?.(); for (const off of offs) if (typeof off === 'function') off(); };
}
