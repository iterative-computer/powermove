// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { createViewerRuntime, formatShapeSize, resolveSelectionGeometry, type ResizeHandle } from './viewer';
import { installKernel } from '../../renderer/src/kernel/install';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/* Selection-tool handles on a parametric shape edit its Size (Width/Height),
   never its Scale; shapes whose Size cannot be edited keep Scale handles. */
function rig(setup?: (PM: any, layer: any) => void) {
  vi.spyOn(window, 'requestAnimationFrame').mockReturnValue(1);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as never);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  PM.proj = PM.mkProject({ w: 1000, h: 600, fps: 30, dur: 4 });
  PM.time = 1;
  const parent = PM.mkLayer('null', { name: 'Rig', p: { 'position.x': 40, rotation: -14, 'scale.x': 120, 'scale.y': 90 } });
  const layer = PM.mkLayer('shape', { name: 'Card', d: { shape: 'rect', w: 200, h: 100, radius: 50, stroke: 0 },
    p: { 'position.x': 400, 'position.y': 250, rotation: 25, skew: 6, 'scale.x': 110, 'anchor.x': -30 } });
  layer.parent = parent.id;
  PM.proj.layers = [layer, parent];
  setup?.(PM, layer);
  PM.ProjectIndex.invalidate(); PM.touch();
  PM.selectLayers([layer.id]);
  let drag: any;
  PM.drag = (_event: Event, options: any) => { drag = options; return { cancel: () => options.cancel?.() }; };
  /* Real layer bounds (they follow the anchor through size changes); no WebGL. */
  Object.assign(PM.GL, { gl: {}, resize: () => true });
  const kernel = installKernel(PM);
  const api = { ...kernel.api('shape-size-check'), scene3d: undefined, space3d: { is3DLayer: () => false } } as any;
  const runtime = createViewerRuntime(api) as any;
  const stage = document.createElement('div');
  stage.innerHTML = '<div id="stage-inner"><canvas id="gl"></canvas></div><canvas id="overlay"></canvas>';
  document.body.append(stage);
  vi.spyOn(stage.querySelector<HTMLElement>('#stage-inner')!, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0 } as DOMRect);
  runtime.attach(stage);
  runtime.shown = 1;
  const geometry = () => resolveSelectionGeometry(api, [PM.L(layer.id)], 1, api.space3d)!;
  const press = (handle: ResizeHandle, dx: number, dy: number, modifiers: { shiftKey?: boolean; altKey?: boolean } = {}) => {
    const point = geometry().handles[handle];
    stage.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, clientX: point.x, clientY: point.y }));
    drag.move(dx, dy, { clientX: point.x + dx, clientY: point.y + dy, ...modifiers });
    return { during: runtime.sizeReadout, up: () => drag.up() };
  };
  const dispose = () => { runtime.dispose(); kernel.uninstall(); };
  return { PM, layer, runtime, geometry, press, dispose };
}

it('drags a corner to change Width/Height under rotation, skew and a parent, keeping the opposite corner and Scale', () => {
  const { PM, layer, runtime, geometry, press, dispose } = rig();
  try {
    const before = geometry().handles;
    const gesture = press('se', 36, 22);
    expect(gesture.during).toMatchObject({ w: expect.any(Number), h: expect.any(Number) });
    gesture.up();
    expect(runtime.sizeReadout ?? null).toBeNull();
    const after = geometry().handles;
    expect(after.nw.x).toBeCloseTo(before.nw.x, 2); expect(after.nw.y).toBeCloseTo(before.nw.y, 2);
    expect(after.se.x).toBeCloseTo(before.se.x + 36, 2); expect(after.se.y).toBeCloseTo(before.se.y + 22, 2);
    const content = PM.resolveContent(PM.L(layer.id), 1);
    expect(content.w).not.toBe(200); expect(content.h).not.toBe(100);
    expect(PM.ev(layer, 'scale.x', 1)).toBe(110); expect(PM.ev(layer, 'scale.y', 1)).toBe(100);
    expect(PM.hist.list().at(-1)).toBe('Resize shape');
    expect(PM.hist.undo()).toBe(true);
    expect(PM.resolveContent(PM.L(layer.id), 1)).toMatchObject({ w: 200, h: 100 });
  } finally { dispose(); }
});

it('Shift keeps proportions and Option resizes from the centre', () => {
  const { PM, layer, geometry, press, dispose } = rig();
  try {
    const centre = (handles: any) => ({ x: (handles.nw.x + handles.se.x) / 2, y: (handles.nw.y + handles.se.y) / 2 });
    const before = centre(geometry().handles);
    press('e', 40, 10, { shiftKey: true, altKey: true }).up();
    const content = PM.resolveContent(PM.L(layer.id), 1);
    expect(content.w / content.h).toBeCloseTo(2, 2);
    expect(content.w).toBeGreaterThan(200);
    const after = centre(geometry().handles);
    expect(after.x).toBeCloseTo(before.x, 2); expect(after.y).toBeCloseTo(before.y, 2);
  } finally { dispose(); }
});

it('keys animated Size and Position at the current time', () => {
  const { PM, layer, press, dispose } = rig((PM, layer) => {
    layer.d.w = PM.P(200, { kf: [PM.KF(0, 200), PM.KF(3, 260)] });
    PM.setKey(layer, 'position.x', 0, 400);
  });
  try {
    press('w', -30, 0).up();
    expect(layer.d.w.kf.map((key: any) => key.t)).toEqual([0, 1, 3]);
    expect(layer.p['position.x'].kf.map((key: any) => key.t)).toEqual([0, 1]);
    expect(PM.ev(layer, 'scale.x', 1)).toBe(110);
  } finally { dispose(); }
});

it('falls back to Scale when Size is driven by an expression', () => {
  const { PM, layer, runtime, press, dispose } = rig((_PM, layer) => { layer.d.w = { v: 200, kf: [], expr: '200 + time * 10' }; });
  try {
    const gesture = press('e', 30, 0);
    expect(runtime.sizeReadout ?? null).toBeNull();
    gesture.up();
    expect(layer.d.w.v).toBe(200);
    expect(PM.ev(layer, 'scale.x', 1)).not.toBe(110);
    expect(PM.hist.list().at(-1)).toBe('Scale layer');
  } finally { dispose(); }
});

it('formats the live dimension badge', () => {
  expect(formatShapeSize({ w: 310, h: 90.04 })).toBe('310 × 90');
  expect(formatShapeSize({ w: 120.25, h: 64.5 })).toBe('120.3 × 64.5');
});
