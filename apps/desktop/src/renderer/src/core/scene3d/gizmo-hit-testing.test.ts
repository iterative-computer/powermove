// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createSceneGizmo, setGizmoMode, setGizmoSpace, type SceneGizmo } from './gizmo';
import { compositionRuntime } from './service';
import { editScene } from './operations';
import { LAYER3D_DEFINITIONS } from './layers';
import { viewportCamera } from './viewport';
import { Vector3 } from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';

// Keep Three's real raycasts and TransformControls; only GPU painting is fake.
vi.mock('three', async () => {
  const actual = await vi.importActual<typeof import('three')>('three');
  return { ...actual, WebGLRenderer: class {
    domElement: HTMLCanvasElement;
    constructor(options: { canvas?: HTMLCanvasElement }) { this.domElement = options.canvas ?? document.createElement('canvas'); }
    setPixelRatio() {} setSize() {} setClearColor() {} clear() {} dispose() {} forceContextLoss() {}
    render(scene: import('three').Scene, camera: import('three').Camera) { camera.updateMatrixWorld(true); scene.updateMatrixWorld(true); }
  } };
});

const controllers: SceneGizmo[] = [];
beforeEach(() => { setGizmoMode('translate'); setGizmoSpace('world'); });
afterEach(() => {
  controllers.splice(0).forEach(controller => controller.dispose());
  document.body.replaceChildren(); vi.restoreAllMocks();
});

function fixture(x = 0, cameraPosition = { x: 0, y: 0, z: 6 }) {
  const frames = new Map<number, FrameRequestCallback>();
  let frameId = 0;
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => { frames.set(++frameId, callback); return frameId; });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => { frames.delete(id); });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 320, h: 180, dur: 2 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  expect(editScene(PM, { operation: 'add_object', object: { id: 'model', p: { x }, source: { primitive: 'box' } } }).ok).toBe(true);
  expect(editScene(PM, { operation: 'add_camera', camera: { p: cameraPosition } }).ok).toBe(true);
  const stage = document.createElement('div'); document.body.append(stage);
  Object.defineProperties(stage, { clientWidth: { value: 320 }, clientHeight: { value: 180 } });
  vi.spyOn(stage, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 320, 180));
  const controller = createSceneGizmo(PM, stage, () => compositionRuntime(PM)); controllers.push(controller);
  controller.update({ x: 0, y: 0, width: 320, height: 180 }, ['model']);
  const paint = () => { const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0)); };
  const handle = (axis: string) => {
    const camera = viewportCamera(PM, compositionRuntime(PM)!.camera).clone();
    if ('aspect' in camera) camera.aspect = 320 / 180;
    camera.updateProjectionMatrix(); camera.updateMatrixWorld(true);
    const center = new Vector3(x, 0, 0).project(camera);
    for (let y = (1 - center.y) * 90; y < 175; y += 2) for (let px = (center.x + 1) * 160 + 20; px < 315; px += 2) {
      if (controller.hover(px, y) && stage.querySelector<HTMLElement>('[data-scene-gizmo]')!.dataset.gizmoAxis === axis) return { x: px, y };
    }
    throw new Error(`No ${axis} handle`);
  };
  return { PM, controller, paint, handle };
}

it.each([
  { x: 0, painted: false },
  { x: 1.5, painted: false },
  { x: 1.5, painted: true },
  { x: 1.5, painted: true, camera: { x: 3, y: 2, z: 6 } }
])('scales with real TransformControls before the next paint ($x / painted: $painted / camera: $camera)', ({ x, painted, camera }) => {
  const f = fixture(x, camera);
  if (painted) { f.paint(); setGizmoMode('rotate'); f.paint(); }
  setGizmoMode('scale');
  const point = f.handle('X');
  expect(f.controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: point.x, clientY: point.y }))).toBe(true);
  window.dispatchEvent(new PointerEvent('pointermove', { clientX: point.x + 25, clientY: point.y }));
  window.dispatchEvent(new PointerEvent('pointerup', { button: 0, clientX: point.x + 25, clientY: point.y }));
  expect(f.PM.L('model').p['scale.x'].v).toBeGreaterThan(100);
  expect(f.PM.L('model').p['scale.y'].v).toBe(100);
  expect(f.PM.L('model').p['scale.z'].v).toBe(100);
  expect(f.PM.L('model').p['position.x'].v).toBe(x);
  const scaled = f.PM.L('model').p['scale.x'].v;
  expect(f.PM.hist.undo()).toBe(true);
  expect(f.PM.L('model').p['scale.x'].v).toBe(100);
  expect(f.PM.hist.redo()).toBe(true);
  expect(f.PM.L('model').p['scale.x'].v).toBe(scaled);
});

it('uses the picked-axis drag plane without waiting for the hover highlight to paint', () => {
  const drag = (paintHover: boolean) => {
    const f = fixture(1.5, { x: 3, y: 2, z: 6 });
    f.paint(); setGizmoMode('scale');
    const point = f.handle('X');
    if (paintHover) f.paint();
    expect(f.controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: point.x, clientY: point.y }))).toBe(true);
    window.dispatchEvent(new PointerEvent('pointermove', { clientX: point.x + 25, clientY: point.y }));
    window.dispatchEvent(new PointerEvent('pointerup', { button: 0, clientX: point.x + 25, clientY: point.y }));
    const value = f.PM.L('model').p['scale.x'].v;
    f.controller.dispose();
    return value;
  };
  expect(drag(false)).toBeCloseTo(drag(true), 6);
});

it('preserves snapping and Escape cancellation when a scale gesture starts before paint', () => {
  const f = fixture(1.5);
  const history = f.PM.hist.list().length;
  setGizmoMode('scale');
  const point = f.handle('X');
  expect(f.controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: point.x, clientY: point.y, ctrlKey: true }))).toBe(true);
  window.dispatchEvent(new PointerEvent('pointermove', { clientX: point.x + 25, clientY: point.y, ctrlKey: true }));
  const scaled = f.PM.L('model').p['scale.x'].v;
  expect(scaled).toBeGreaterThan(100);
  expect(scaled / 10).toBeCloseTo(Math.round(scaled / 10), 6);
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
  expect(f.controller.active()).toBe(false);
  expect(f.PM.L('model').p['scale.x'].v).toBe(100);
  expect(f.PM.hist.list().length).toBe(history);
});
