// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSceneGizmo, setGizmoMode, setGizmoSpace, type SceneGizmo } from './gizmo';
import { compositionRuntime } from './service';
import { editScene } from './operations';
import { LAYER3D_DEFINITIONS } from './layers';
import { makePM } from '../../legacy/__tests__/make-pm';

const gpu = vi.hoisted(() => ({ renderers: [] as any[], controls: [] as any[], failRenderer: false, failControls: false,
  failHelper: false, contextEvents: 0 }));
vi.mock('three', async () => {
  const actual = await vi.importActual<typeof import('three')>('three');
  return { ...actual, WebGLRenderer: class {
    domElement: HTMLCanvasElement;
    contextEvents = ['webglcontextlost', 'webglcontextrestored', 'webglcontextcreationerror'];
    onContext = () => { gpu.contextEvents++; };
    setPixelRatio = vi.fn(); setSize = vi.fn(); setClearColor = vi.fn(); clear = vi.fn(); render = vi.fn();
    dispose = vi.fn(() => { for (const event of this.contextEvents) this.domElement.removeEventListener(event, this.onContext); });
    forceContextLoss = vi.fn();
    constructor(options: { canvas?: HTMLCanvasElement }) {
      this.domElement = options.canvas ?? document.createElement('canvas'); gpu.renderers.push(this);
      for (const event of this.contextEvents) this.domElement.addEventListener(event, this.onContext);
      if (gpu.failRenderer) { gpu.failRenderer = false; throw new Error('GPU setup failed'); }
    }
  } };
});
vi.mock('three/addons/controls/TransformControls.js', async () => {
  const actual = await vi.importActual<typeof import('three')>('three');
  return { TransformControls: class extends actual.Object3D {
    object: import('three').Object3D | null = null;
    domElement: HTMLElement | null;
    axis: string | null = null; dragging = false; camera: import('three').Camera;
    helper = new actual.Object3D();
    disconnect = vi.fn(); dispose = vi.fn();
    setTranslationSnap = vi.fn(); setRotationSnap = vi.fn(); setScaleSnap = vi.fn();
    setMode = vi.fn(); setSpace = vi.fn();
    constructor(camera: import('three').Camera, domElement: HTMLElement | null = null) {
      super(); this.camera = camera; this.domElement = domElement; gpu.controls.push(this);
      if (gpu.failControls) { gpu.failControls = false; throw new Error('Control setup failed'); }
    }
    getHelper() {
      if (gpu.failHelper) { gpu.failHelper = false; throw new Error('Helper setup failed'); }
      return this.helper;
    }
    attach(object: import('three').Object3D) { this.object = object; return this; }
    detach() { this.object = null; return this; }
    pointerHover() { this.axis = this.object ? 'X' : null; }
    pointerDown() { this.dragging = !!this.object; }
    pointerMove() {
      if (!this.dragging || !this.object) return;
      this.object.position.x += 1; this.dispatchEvent({ type: 'objectChange' } as any);
    }
    pointerUp() { this.dragging = false; }
  } };
});

const controllers: SceneGizmo[] = [];
beforeEach(() => {
  vi.useFakeTimers(); gpu.renderers.length = 0; gpu.controls.length = 0;
  gpu.failRenderer = gpu.failControls = gpu.failHelper = false; gpu.contextEvents = 0;
  setGizmoMode('translate'); setGizmoSpace('world');
});
afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose();
  document.body.replaceChildren(); vi.useRealTimers(); vi.restoreAllMocks();
});

function fixture() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 320, h: 180, dur: 2 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  expect(editScene(PM, { operation: 'add_object', object: { id: 'model', source: { primitive: 'box' } } }).ok).toBe(true);
  const plain = PM.mkLayer('solid'); PM.proj.layers.push(plain); PM.touch();
  const stage = document.createElement('div'); document.body.append(stage);
  const runtime = vi.fn(() => compositionRuntime(PM));
  const controller = createSceneGizmo(PM, stage, runtime); controllers.push(controller);
  const canvas = stage.querySelector<HTMLCanvasElement>('[data-scene-gizmo]')!;
  const rect = { x: 0, y: 0, width: 320, height: 180 };
  const press = () => controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: 10, clientY: 20 }));
  const move = () => window.dispatchEvent(new PointerEvent('pointermove', { clientX: 20, clientY: 20 }));
  return { PM, stage, canvas, plain, runtime, controller, rect, press, move };
}

describe('on-demand 3D gizmo resources', () => {
  it('keeps the complete controller/canvas contract without GPU allocations for empty or 2D selections', () => {
    const f = fixture();
    expect(f.canvas.hidden).toBe(true); expect(f.canvas.getAttribute('aria-hidden')).toBe('true');
    for (const ids of [[], [f.plain.id], ['missing'], ['model', f.plain.id]]) f.controller.update(f.rect, ids);
    f.controller.update(f.rect); setGizmoMode('rotate'); setGizmoSpace('local');
    expect(f.controller.covers(['model'])).toBe(true); expect(f.controller.covers([f.plain.id])).toBe(false);
    expect(f.controller.hover(10, 20)).toBe(false); expect(f.press()).toBe(false); expect(f.controller.active()).toBe(false);
    f.controller.cancel(); vi.runAllTimers();
    expect(gpu.renderers).toHaveLength(0); expect(gpu.controls).toHaveLength(0); expect(f.runtime).not.toHaveBeenCalled();
    f.controller.dispose(); f.controller.dispose(); f.controller.update(f.rect, ['model']);
    expect(f.stage.querySelector('[data-scene-gizmo]')).toBeNull(); expect(gpu.renderers).toHaveLength(0);
  });

  it('initializes synchronously on the first 3D update and retains the idle/reselection behavior', () => {
    const f = fixture(); const originalCanvas = f.canvas;
    setGizmoMode('rotate'); setGizmoSpace('local');
    f.controller.update(f.rect, ['model']);
    expect(gpu.renderers).toHaveLength(1); expect(gpu.controls).toHaveLength(1); expect(f.runtime).toHaveBeenCalledOnce();
    expect(gpu.renderers[0].domElement).toBe(originalCanvas); expect(f.stage.querySelectorAll('[data-scene-gizmo]')).toHaveLength(1);
    expect(gpu.controls[0].object).not.toBeNull(); expect(gpu.controls[0].setMode).toHaveBeenLastCalledWith('rotate');
    expect(gpu.controls[0].setSpace).toHaveBeenLastCalledWith('local'); expect(f.canvas.hidden).toBe(false);
    vi.runAllTimers(); const draws = gpu.renderers[0].render.mock.calls.length;
    f.controller.update(f.rect, []); const reads = f.runtime.mock.calls.length;
    for (let i = 0; i < 12; i++) f.controller.update(f.rect, []);
    vi.runAllTimers();
    expect(f.canvas.hidden).toBe(true); expect(f.runtime.mock.calls.length).toBe(reads);
    expect(gpu.renderers[0].render.mock.calls.length).toBe(draws);
    f.controller.update(f.rect, ['model']);
    expect(f.canvas.hidden).toBe(false); expect(gpu.renderers).toHaveLength(1); expect(gpu.controls).toHaveLength(1);
  });

  it('preserves drag commit, cancellation and undo after the deferred initialization', () => {
    const f = fixture(), before = f.PM.hist.list().length;
    f.controller.update(f.rect, ['model']);
    expect(f.press()).toBe(true); expect(f.controller.active()).toBe(true); f.move();
    expect(f.PM.L('model').p['position.x'].v).toBe(1);
    window.dispatchEvent(new PointerEvent('pointerup', { clientX: 20, clientY: 20 }));
    expect(f.controller.active()).toBe(false); expect(f.PM.hist.list()).toHaveLength(before + 1);
    f.PM.hist.undo(); expect(f.PM.L('model').p['position.x'].v).toBe(0);
    const afterUndo = f.PM.hist.list();
    f.controller.update(f.rect, ['model']); expect(f.press()).toBe(true); f.move();
    expect(f.PM.L('model').p['position.x'].v).toBe(1); f.controller.cancel();
    expect(f.PM.L('model').p['position.x'].v).toBe(0); expect(f.PM.hist.list()).toEqual(afterUndo);
  });

  it('cancels an active drag on disposal, releases resources once and allows a fresh controller', () => {
    const f = fixture(); f.controller.update(f.rect, ['model']); expect(f.press()).toBe(true); f.move();
    f.controller.dispose(); f.controller.dispose();
    expect(f.PM.L('model').p['position.x'].v).toBe(0); expect(f.controller.active()).toBe(false);
    expect(gpu.renderers[0].dispose).toHaveBeenCalledOnce(); expect(gpu.renderers[0].forceContextLoss).toHaveBeenCalledOnce();
    expect(gpu.controls[0].dispose).toHaveBeenCalledOnce(); expect(f.canvas.isConnected).toBe(false);
    f.move(); expect(f.PM.L('model').p['position.x'].v).toBe(0);
    const next = createSceneGizmo(f.PM, f.stage, f.runtime); controllers.push(next);
    expect(gpu.renderers).toHaveLength(1); next.update(f.rect, ['model']); expect(gpu.renderers).toHaveLength(2);
    expect(f.stage.querySelectorAll('[data-scene-gizmo]')).toHaveLength(1);
  });

  it('reuses the live context after control setup fails and releases it when no retry occurs', () => {
    const f = fixture(); gpu.failControls = true;
    expect(() => f.controller.update(f.rect, ['model'])).toThrow('Control setup failed');
    expect(gpu.renderers[0].dispose).not.toHaveBeenCalled(); expect(gpu.renderers[0].forceContextLoss).not.toHaveBeenCalled();
    expect(f.canvas.hidden).toBe(true); expect(f.controller.hover(10, 20)).toBe(false);
    f.controller.update(f.rect, ['model']);
    expect(gpu.renderers).toHaveLength(1); expect(f.canvas.hidden).toBe(false);
    expect(gpu.renderers[0].domElement).toBe(f.canvas); expect(f.stage.querySelectorAll('[data-scene-gizmo]')).toHaveLength(1);
    f.controller.dispose();
    expect(gpu.renderers[0].dispose).toHaveBeenCalledOnce(); expect(gpu.renderers[0].forceContextLoss).toHaveBeenCalledOnce();

    const abandoned = fixture(); gpu.failControls = true;
    expect(() => abandoned.controller.update(abandoned.rect, ['model'])).toThrow('Control setup failed');
    abandoned.controller.dispose(); abandoned.controller.dispose();
    expect(gpu.renderers[1].dispose).toHaveBeenCalledOnce(); expect(gpu.renderers[1].forceContextLoss).toHaveBeenCalledOnce();
    expect(abandoned.canvas.isConnected).toBe(false);
  });

  it('removes only a failed renderer constructor’s context listeners and restores the original canvas method', () => {
    const f = fixture(), external = vi.fn();
    const add = vi.fn(f.canvas.addEventListener.bind(f.canvas));
    Object.defineProperty(f.canvas, 'addEventListener', { configurable: true, writable: true, value: add });
    f.canvas.addEventListener('webglcontextlost', external);
    gpu.failRenderer = true;
    expect(() => f.controller.update(f.rect, ['model'])).toThrow('GPU setup failed');
    expect(f.canvas.addEventListener).toBe(add); expect(gpu.controls).toHaveLength(0);
    f.canvas.dispatchEvent(new Event('webglcontextlost'));
    expect(gpu.contextEvents).toBe(0); expect(external).toHaveBeenCalledOnce();
    f.controller.update(f.rect, ['model']);
    expect(gpu.renderers).toHaveLength(2); expect(f.canvas.hidden).toBe(false);
    f.canvas.dispatchEvent(new Event('webglcontextlost'));
    expect(gpu.contextEvents).toBe(1); expect(external).toHaveBeenCalledTimes(2);
    expect(f.canvas.addEventListener).toBe(add);
    f.controller.dispose(); f.canvas.dispatchEvent(new Event('webglcontextlost'));
    expect(gpu.contextEvents).toBe(1); expect(external).toHaveBeenCalledTimes(3);
  });

  it('disposes controls after later setup failure and keeps the renderer usable for retry', () => {
    const f = fixture(); gpu.failHelper = true;
    expect(() => f.controller.update(f.rect, ['model'])).toThrow('Helper setup failed');
    expect(gpu.controls[0].dispose).toHaveBeenCalledOnce(); expect(gpu.controls[0].domElement).toBe(f.canvas);
    expect(gpu.renderers[0].dispose).not.toHaveBeenCalled();
    f.controller.update(f.rect, ['model']);
    expect(gpu.renderers).toHaveLength(1); expect(gpu.controls).toHaveLength(2); expect(f.canvas.hidden).toBe(false);
    f.controller.dispose();
    expect(gpu.controls[0].dispose).toHaveBeenCalledOnce(); expect(gpu.controls[1].dispose).toHaveBeenCalledOnce();
  });

  it('rolls back mode and window subscriptions if listener installation fails', () => {
    const f = fixture(), add = window.addEventListener, remove = vi.spyOn(window, 'removeEventListener');
    const installed = vi.spyOn(window, 'addEventListener').mockImplementation((type, callback, options) => {
      if (type === 'keyup') throw new Error('Listener setup failed');
      add.call(window, type, callback, options);
    });
    expect(() => f.controller.update(f.rect, ['model'])).toThrow('Listener setup failed');
    expect(remove).toHaveBeenCalledWith('keydown', expect.any(Function), true);
    setGizmoMode('rotate'); vi.runAllTimers();
    expect(f.runtime).not.toHaveBeenCalled(); expect(gpu.controls[0].dispose).toHaveBeenCalledOnce();
    installed.mockRestore(); f.controller.update(f.rect, ['model']);
    expect(gpu.renderers).toHaveLength(1); expect(f.canvas.hidden).toBe(false);
    expect(gpu.controls[1].setMode).toHaveBeenLastCalledWith('rotate');
  });
});
