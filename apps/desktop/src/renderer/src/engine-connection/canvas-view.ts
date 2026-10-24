import type { PMRegistry } from '../legacy/registry';
import type { EditorCamera } from '../../../extensions/viewer/engine-present/protocol';
import { onViewportChange, viewportEngineCamera } from '../core/scene3d/viewport';

/** The built-in viewer's 3D rule: a composition with 3D objects shows the 3D editor. */
export function compositionHas3D(PM: PMRegistry): boolean {
  const comp = (PM as any).curComp?.() ?? (PM as any).proj;
  return !!comp?.layers?.some((layer: any) => layer?.d?.definition === 'powermove.3d.object');
}

/** The editor camera in the Editor view; null in the Camera view. Purely
 *  editor state: it never asks the engine for render-camera facts. */
export function engineCanvasEditorCamera(PM: PMRegistry): EditorCamera | null {
  return (PM as any).proj ? viewportEngineCamera(PM) : null;
}

/** The view a bound pre-engine canvas presents: the built-in viewer's. */
export function engineCanvasViewParams(PM: PMRegistry): { camera?: EditorCamera } {
  const camera = engineCanvasEditorCamera(PM);
  return camera ? { camera } : {};
}

/** Refreshes on each 3D view change, coalesced into one microtask. */
export function watchEngineCanvasView(PM: PMRegistry, refresh: () => void): () => void {
  let live = true, queued = false;
  const off = onViewportChange(PM, () => {
    if (queued || !live) return;
    queued = true;
    queueMicrotask(() => { queued = false; if (live) refresh(); });
  });
  return () => { live = false; off(); };
}
