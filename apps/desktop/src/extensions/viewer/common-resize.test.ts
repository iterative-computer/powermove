// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { createViewerRuntime, resolveSelectionGeometry } from './viewer';
import { installKernel } from '../../renderer/src/kernel/install';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it.each([false, true])('resizes a shape root and its selected child together without editing the child twice (animated=%s)', animated => {
  vi.spyOn(window, 'requestAnimationFrame').mockReturnValue(1);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as never);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 1000, h: 600, fps: 30, dur: 4 });
  PM.time = 1;
  const parent = PM.mkLayer('shape', { name: 'Shape parent', d: { w: 100, h: 100 }, p: { 'position.x': 200, 'position.y': 200 } });
  const child = PM.mkLayer('shape', { name: 'Selected child', d: { w: 20, h: 20 }, p: { 'position.x': 75, 'position.y': 0 } });
  child.parent = parent.id;
  const other = PM.mkLayer('shape', { name: 'Other root', d: { w: 100, h: 100 }, p: { 'position.x': 400, 'position.y': 200 } });
  PM.proj.layers = [parent, child, other];
  PM.ProjectIndex.invalidate();
  PM.selectLayers([parent.id, child.id, other.id]);
  if (animated) {
    PM.setKey(parent, 'scale.x', 0, 100);
    PM.setKey(parent, 'scale.y', 0, 100);
  }
  const childBefore = JSON.stringify(child);
  let drag: any;
  PM.drag = (_event: Event, options: any) => { drag = options; return { cancel: () => options.cancel?.() }; };
  PM.GL = {
    gl: {},
    bounds(layer: any, time: number) {
      const content = PM.resolveContent(layer, time);
      return { x0: -content.w / 2, y0: -content.h / 2, x1: content.w / 2, y1: content.h / 2 };
    },
    resize: () => true,
  };
  const kernel = installKernel(PM);
  const api = { ...kernel.api('common-resize-check'), scene3d: undefined, space3d: { is3DLayer: () => false } } as any;
  const runtime = createViewerRuntime(api);
  const stage = document.createElement('div');
  stage.innerHTML = '<div id="stage-inner"><canvas id="gl"></canvas></div><canvas id="overlay"></canvas>';
  document.body.append(stage);
  const inner = stage.querySelector<HTMLElement>('#stage-inner')!;
  vi.spyOn(inner, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0 } as DOMRect);
  runtime.attach(stage);
  runtime.shown = 1;
  try {
    const selection = resolveSelectionGeometry(api, [parent, child, other], 1, api.space3d)!;
    expect(selection.mode).toBe('common');
    expect(selection.roots.map(layer => layer.id)).toEqual([parent.id, other.id]);
    const before = PM.worldMatrix(child, 1);
    const pivot = selection.handles.nw, handle = selection.handles.se;
    stage.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, clientX: handle.x, clientY: handle.y }));
    expect(drag).toBeDefined();
    drag.move(handle.x - pivot.x, handle.y - pivot.y, {
      clientX: pivot.x + (handle.x - pivot.x) * 2,
      clientY: pivot.y + (handle.y - pivot.y) * 2,
    });
    drag.up();
    const after = PM.worldMatrix(child, 1);
    expect(after[0]).toBeCloseTo(before[0] * 2);
    expect(after[3]).toBeCloseTo(before[3] * 2);
    expect(after[4]).toBeCloseTo(pivot.x + (before[4] - pivot.x) * 2);
    expect(after[5]).toBeCloseTo(pivot.y + (before[5] - pivot.y) * 2);
    expect(JSON.stringify(child)).toBe(childBefore);
    expect(PM.resolveContent(parent, 1)).toMatchObject({ w: 100, h: 100 });
    expect(PM.ev(parent, 'scale.x', 1)).toBe(200);
    expect(parent.p['scale.x'].kf).toHaveLength(animated ? 2 : 0);
    expect(PM.resolveContent(other, 1)).toMatchObject({ w: 200, h: 200 });
    expect(PM.ev(other, 'scale.x', 1)).toBe(100);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.worldMatrix(PM.L(child.id), 1)).toEqual(before);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.worldMatrix(PM.L(child.id), 1)).toEqual(after);
  } finally {
    runtime.dispose(); kernel.uninstall();
  }
});
