import * as THREE from 'three';
import { compositionRuntime } from '../../src/renderer/src/core/scene3d/service';
import { fitCamera } from '../../src/renderer/src/core/scene3d/modal';
import { ViewportOverlay, type OverlayFrame } from '../../src/renderer/src/core/scene3d/overlay';

type Capture = { x: number; y: number; width: number; height: number; outputWidth: number; outputHeight: number };

/** Test-only S7 MSAA edge witness, invoked solely while EngineDoc is disabled. */
(window as any).__chromeReferenceEdges = (capture: Capture) => {
  const PM = (window as any).PM;
  if (PM.EngineDoc?.enabled !== false) throw new Error('MSAA reference requires the explicit legacy test branch');
  const viewer = PM.Kernel.services.get('viewer');
  const stage = viewer.stage as HTMLElement, inner = viewer.inner as HTMLElement;
  const overlay = stage.querySelector('[data-scene-overlay]') as HTMLCanvasElement | null;
  if (!overlay || !inner) throw new Error('Viewport geometry is unavailable');
  const stageBox = stage.getBoundingClientRect();
  const fullWidth = overlay.width, fullHeight = overlay.height;
  // Use the same unrounded geometry as Viewer.drawOverlay. DOM layout
  // quantizes inner widths/offsets before getBoundingClientRect returns them.
  const width = PM.proj.w * viewer.shown, height = PM.proj.h * viewer.shown;
  const rect = { x: (stageBox.width - width) / 2 + (viewer.fit ? 0 : viewer.pan[0]),
    y: (stageBox.height - height) / 2 + (viewer.fit ? 0 : viewer.pan[1]), width, height };
  // Read the app's actual transient camera. This helper is bundled in a
  // separate module realm, so importing viewport.ts would create a second
  // WeakMap and silently witness the initial shot camera instead.
  const snapshot = PM.Kernel.api('chrome-msaa-reference').scene3d.viewport.state();
  const facts = snapshot.engineCamera;
  if (snapshot.view.mode !== 'editor' || facts?.projection.kind !== 'perspective' || facts.projection.near <= .001)
    throw new Error('This witness requires the fixture editor perspective');
  const lens = facts.projection;
  const sourceCamera = new THREE.PerspectiveCamera(lens.fovYDegrees, rect.width / rect.height, lens.near, lens.far);
  sourceCamera.matrix.copy(new THREE.Matrix4().fromArray(facts.view).invert());
  sourceCamera.matrix.decompose(sourceCamera.position, sourceCamera.quaternion, sourceCamera.scale);
  sourceCamera.updateMatrixWorld(true);
  const camera = fitCamera(sourceCamera, rect);
  const view = snapshot.view;
  // configure() uses distance * .002 for perspective near (unclamped here).
  const distance = lens.near / .002;
  const frame: OverlayFrame = { camera, rect, runtime: compositionRuntime(PM),
    userView: view.mode === 'editor', axis: view.axis, distance, fov: view.fov,
    overlays: true, xray: snapshot.xray, selected: new Set(), active: new Set(), glyphs: [], guides: [], gizmo: null };
  const fakeStage = { clientWidth: stage.clientWidth, clientHeight: stage.clientHeight, append() {} } as unknown as HTMLElement;
  // Witness the actual legacy depth+grid pass. MSAA changes coverage where
  // the grid intersects the interior of a model as well as its silhouette.
  // The grid uses the unchanged production shader in both reference renders.
  const render = (antialias: boolean): { pixels: Uint8Array; samples: number } => {
    const witness = new ViewportOverlay(fakeStage);
    const internals = witness as any;
    if (!antialias) {
      internals.renderer.dispose(); internals.renderer.forceContextLoss();
      internals.renderer = new THREE.WebGLRenderer({ alpha: true, antialias: false, preserveDrawingBuffer: true });
      internals.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
      internals.renderer.autoClear = false;
      internals.canvas = internals.renderer.domElement;
    }
    try {
      witness.draw(frame);
      const gl = internals.renderer.getContext() as WebGL2RenderingContext;
      const samples = gl.getParameter(gl.SAMPLES) as number;
      const pixels = new Uint8Array(fullWidth * fullHeight * 4);
      if (witness.canvas.width !== fullWidth || witness.canvas.height !== fullHeight)
        throw new Error('MSAA witness must match the overlay drawing buffer');
      gl.readPixels(0, 0, fullWidth, fullHeight, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      return { pixels, samples };
    } finally { witness.dispose(); }
  };
  const antialiasedResult = render(true), singleResult = render(false);
  const antialiased = antialiasedResult.pixels, single = singleResult.pixels;
  const samples = antialiasedResult.samples;
  if (samples < 2 || singleResult.samples !== 0) throw new Error(`Invalid MSAA witness samples ${samples}/${singleResult.samples}`);
  // The exact S7 witness: pixels changed by MSAA in the reference depth/grid pass,
  // dilated once. This does not use the native output to decide exclusions.
  const raw = new Uint8Array(fullWidth * fullHeight);
  for (let i = 0; i < raw.length; i++) {
    const j = i * 4;
    if (single[j] !== antialiased[j] || single[j + 1] !== antialiased[j + 1] ||
        single[j + 2] !== antialiased[j + 2] || single[j + 3] !== antialiased[j + 3]) raw[i] = 1;
  }
  // Every non-X-ray fixture here has a grid intersecting visible geometry.
  // A detached/initial camera can otherwise silently produce an empty mask.
  if (!snapshot.xray && !raw.some(value => value !== 0))
    throw new Error('The occluding reference fixture must have witnessed MSAA edges');
  const grown = raw.slice();
  for (let y = 0; y < fullHeight; y++) for (let x = 0; x < fullWidth; x++) {
    if (!raw[y * fullWidth + x]) continue;
    for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
      const px = x + dx, py = y + dy;
      if (px >= 0 && px < fullWidth && py >= 0 && py < fullHeight) grown[py * fullWidth + px] = 1;
    }
  }
  // readRenderTargetPixels is bottom-up; capturePage is top-down and is at
  // device resolution. Map the exact CSS crop through stage geometry, then
  // flip its Y index into the readback buffer.
  const mask = new Uint8Array(capture.outputWidth * capture.outputHeight);
  for (let y = 0; y < capture.outputHeight; y++) for (let x = 0; x < capture.outputWidth; x++) {
    const cssX = capture.x + (x + .5) * capture.width / capture.outputWidth;
    const cssY = capture.y + (y + .5) * capture.height / capture.outputHeight;
    const sx = Math.floor((cssX - stageBox.left) * fullWidth / stageBox.width);
    const sy = Math.floor((cssY - stageBox.top) * fullHeight / stageBox.height);
    if (sx >= 0 && sx < fullWidth && sy >= 0 && sy < fullHeight)
      mask[y * capture.outputWidth + x] = grown[(fullHeight - 1 - sy) * fullWidth + sx]!;
  }
  let binary = '';
  for (let i = 0; i < mask.length; i += 0x8000) binary += String.fromCharCode(...mask.subarray(i, i + 0x8000));
  return { samples, width: fullWidth, height: fullHeight, mask: btoa(binary) };
};
