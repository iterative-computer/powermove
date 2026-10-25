/** Inspect the 3D controls drawn by main's composition viewer. This bundle is
 * evaluated in the app window so it reads that window's viewport state. */
(window as any).__chromeReferenceState = () => {
  const PM = (window as any).PM;
  const viewer = PM.Kernel.services.get('viewer');
  const api = PM.Kernel.api('chrome-reference').scene3d;
  const canvas = viewer.stage?.querySelector('canvas[data-scene-gizmo]') as HTMLCanvasElement | null;
  const presentation = viewer.enginePresentation;
  return {
    mode: api.getView(),
    gizmoVisible: !!canvas && !canvas.hidden && getComputedStyle(canvas).visibility !== 'hidden',
    backing: canvas ? [canvas.width, canvas.height] : null,
    frameViewSeq: presentation?.latest?.viewSeq ?? null,
    stateViewSeq: presentation?.state?.viewSeq ?? null,
    camera: presentation?.state?.view?.camera ?? null,
    presenting: !!presentation?.presenting,
  };
};
