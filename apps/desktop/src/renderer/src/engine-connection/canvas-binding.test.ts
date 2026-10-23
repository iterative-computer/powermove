// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import type { EngineViewerService, ViewParams } from '../../../extensions/viewer/engine-present/presentation';
import { ENGINE_SELF_PRESENTS_ATTRIBUTE as PRESENTATION_SELF_PRESENTS, ENGINE_STAGE_EVENT as PRESENTATION_STAGE_EVENT } from '../../../extensions/viewer/engine-present/presentation';
import { ENGINE_SELF_PRESENTS_ATTRIBUTE, ENGINE_STAGE_EVENT, EngineCanvasBinding, watchRemoval, type BoundPresentation } from './canvas-binding';

class FakePresentation implements BoundPresentation {
  readonly exclusive = true;
  updates: Array<{ params: ViewParams; stage?: { width: number; height: number } }> = [];
  disposed = false;
  constructor(readonly stage: HTMLElement, readonly before: Element | null) {}
  update(params: ViewParams, stage?: { width: number; height: number }): void {
    this.updates.push({ params, ...(stage ? { stage } : {}) });
  }
  dispose(): void {
    this.disposed = true;
  }
}

function rect(el: Element, r: { left: number; top: number; width: number; height: number }): void {
  (el as HTMLElement).getBoundingClientRect = () => ({ ...r, x: r.left, y: r.top, right: r.left + r.width, bottom: r.top + r.height, toJSON: () => r }) as DOMRect;
}

function setup(view?: () => Partial<ViewParams>) {
  const made: FakePresentation[] = [];
  const service = { exclusive: true } as unknown as EngineViewerService;
  let enabled = true;
  const binding = new EngineCanvasBinding({
    service: () => (enabled ? service : null),
    composition: () => ({ w: 1920, h: 1080 }),
    quality: () => 0.5,
    view,
    present: ({ stage, before }) => {
      const p = new FakePresentation(stage, before);
      made.push(p);
      return p;
    }
  });
  const stage = document.createElement('div');
  const canvas = document.createElement('canvas');
  const overlay = document.createElement('canvas');
  stage.append(canvas, overlay);
  document.body.append(stage);
  rect(stage, { left: 0, top: 0, width: 1000, height: 600 });
  rect(canvas, { left: 20, top: 30, width: 960, height: 540 });
  return { binding, made, stage, canvas, overlay, disable: () => (enabled = false) };
}

describe('EngineCanvasBinding (render.gl.init/resize with the engine on)', () => {
  it('refreshes a fork’s camera and shading on its existing presenter without changing its canvas crop', () => {
    let shading: ViewParams['shading'] = 'solid';
    const camera = { view: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -2, -3, -4, 1], projection: { kind: 'orthographic', halfHeight: 3, near: .01, far: 100 } } as NonNullable<ViewParams['camera']>;
    const { binding, made, canvas } = setup(() => ({ camera, shading, xray: true, localView: ['cube'] }));
    binding.bind(canvas);
    binding.resize(960, 540, { x: 480, y: 270, width: 480, height: 270 });
    const crop = made[0]!.updates.at(-1)!.params.canvas;
    shading = 'wireframe';
    binding.refresh();
    expect(made).toHaveLength(1);
    expect(binding.viewerPresentation).toBe(made[0]);
    expect(made[0]!.updates.at(-1)!.params).toMatchObject({ camera, shading: 'wireframe', xray: true, localView: ['cube'], canvas: crop });
  });

  it('presents where the passed canvas sits, under the caller overlay, and hides the canvas', () => {
    const { binding, made, stage, canvas, overlay } = setup();
    expect(ENGINE_STAGE_EVENT).toBe(PRESENTATION_STAGE_EVENT);
    expect(binding.bind(canvas)).toBe(true);
    expect(made).toHaveLength(0);
    binding.resize();
    expect(made).toHaveLength(1);
    const p = made[0]!;
    expect(p.stage).toBe(stage);
    // Right after the passed canvas: the caller's overlay stays on top.
    expect(p.before).toBe(overlay);
    expect(canvas.style.visibility).toBe('hidden');
    // 960 CSS px for 1920 composition px; centered 20 px right, 30 px down of center.
    expect(p.updates[0]).toEqual({ params: { zoom: 0.5, pan: [0, 0], resolution: 'half', media: 'automatic', playEveryFrame: false }, stage: { width: 1000, height: 600 } });
    rect(canvas, { left: 100, top: 30, width: 480, height: 270 });
    binding.resize();
    expect(made).toHaveLength(1);
    expect(p.updates[1]!.params).toMatchObject({ zoom: 0.25, pan: [-160, -135] });
    binding.unbind();
    expect(p.disposed).toBe(true);
    expect(canvas.style.visibility).toBe('');
  });

  it('never competes with a viewer that presents by itself', () => {
    const { binding, made, stage, canvas } = setup();
    binding.bind(canvas);
    binding.resize();
    expect(made).toHaveLength(1);
    // The built-in viewer's own presentation starts over the same stage.
    stage.dataset['engineStage'] = 'viewer';
    canvas.style.visibility = 'hidden';
    stage.dispatchEvent(new Event(ENGINE_STAGE_EVENT, { bubbles: true }));
    expect(made[0]!.disposed).toBe(true);
    // It hid its canvas itself; the binding leaves that alone.
    expect(canvas.style.visibility).toBe('hidden');
    binding.resize();
    expect(made).toHaveLength(1);
  });

  it('never presents over a viewer that presents by itself, before its presentation exists or after it is gone', () => {
    const { binding, made, stage, canvas } = setup();
    expect(ENGINE_SELF_PRESENTS_ATTRIBUTE).toBe(PRESENTATION_SELF_PRESENTS);
    // The built-in viewer marks its stage at mount, before render.gl.init;
    // its presentation comes once the engine connection is up.
    stage.setAttribute(ENGINE_SELF_PRESENTS_ATTRIBUTE, '');
    binding.bind(canvas);
    binding.resize();
    expect(made).toHaveLength(0);
    expect(canvas.style.visibility).toBe('');
    // It unmounts (disabled, say) and its WebGL canvas stays in the panel.
    stage.removeAttribute(ENGINE_SELF_PRESENTS_ATTRIBUTE);
    expect(made).toHaveLength(0);
    // An old fork that mounts on the same skeleton and draws through the
    // compositor gets the binding.
    binding.resize();
    expect(made).toHaveLength(1);
  });

  it('one binding per window: a new canvas replaces the old; a detached canvas unbinds', () => {
    const { binding, made, canvas } = setup();
    binding.bind(canvas);
    binding.resize();
    const other = document.createElement('canvas');
    document.body.append(other);
    binding.bind(other);
    expect(made[0]!.disposed).toBe(true);
    expect(binding.bound).toBe(other);
    other.remove();
    binding.resize();
    expect(binding.bound).toBeNull();
  });

  it('passes the caller’s canvas: backing size, crop and CSS box (decision V1)', () => {
    const { binding, made, canvas } = setup();
    binding.bind(canvas);
    binding.resize(960, 540);
    // 1920 × 1080 in a 960 × 540 box at (20, 30) of the stage.
    expect(made[0]!.updates.at(-1)!.params.canvas).toEqual({
      width: 960, height: 540, crop: { x: 0, y: 0, width: 1920, height: 1080 }, css: { x: 20, y: 30, width: 960, height: 540 }
    });
    // A cropped preview viewport: the box is the crop's.
    binding.resize(1920, 1080, { x: 480, y: 270, width: 480, height: 270 });
    expect(made[0]!.updates.at(-1)!.params.canvas).toEqual({
      width: 1920, height: 1080, crop: { x: 480, y: 270, width: 480, height: 270 }, css: { x: 20, y: 30, width: 960, height: 540 }
    });
    // Without a size (an old caller): the engine draws at zoom × DPR.
    binding.resize();
    expect(made[0]!.updates.at(-1)!.params).not.toHaveProperty('canvas');
  });

  it('a caller that tears its canvas down without another resize leaves no presenter behind', () => {
    const watched: Array<{ canvas: HTMLCanvasElement; changed: () => void; stopped: boolean }> = [];
    const made: FakePresentation[] = [];
    const binding = new EngineCanvasBinding({
      service: () => ({ exclusive: true }) as unknown as EngineViewerService,
      composition: () => ({ w: 1920, h: 1080 }),
      quality: () => 1,
      present: ({ stage, before }) => {
        const p = new FakePresentation(stage, before);
        made.push(p);
        return p;
      },
      watch: (canvas, changed) => {
        const entry = { canvas, changed, stopped: false };
        watched.push(entry);
        return () => { entry.stopped = true; };
      }
    });
    const stage = document.createElement('div');
    const canvas = document.createElement('canvas');
    stage.append(canvas);
    document.body.append(stage);
    rect(stage, { left: 0, top: 0, width: 1000, height: 600 });
    rect(canvas, { left: 20, top: 30, width: 960, height: 540 });
    binding.bind(canvas);
    binding.resize();
    expect(watched).toHaveLength(1);
    // A size change while connected is the caller's to report.
    watched[0]!.changed();
    expect(made[0]!.disposed).toBe(false);
    // A disabled viewer extension (say) removes its stage and calls nothing more.
    stage.remove();
    watched[0]!.changed();
    expect(made[0]!.disposed).toBe(true);
    expect(binding.bound).toBeNull();
    expect(binding.presenting).toBe(false);
    expect(watched[0]!.stopped).toBe(true);
  });

  it('the default watch reports a 0x0 canvas whose ancestor is removed', async () => {
    const stage = document.createElement('div');
    const canvas = document.createElement('canvas');
    canvas.width = 0;
    canvas.height = 0;
    stage.append(canvas);
    document.body.append(stage);
    let calls = 0;
    const stop = watchRemoval(canvas, () => { calls += 1; });
    // An unrelated mutation while the canvas stays connected reports nothing.
    document.body.append(document.createElement('span'));
    await Promise.resolve();
    expect(calls).toBe(0);
    stage.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(calls).toBeGreaterThan(0);
    const seen = calls;
    stop();
    document.body.append(stage);
    stage.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(calls).toBe(seen);
  });

  it('a bound 0x0 canvas removed without another resize leaves no presenter behind', async () => {
    const made: FakePresentation[] = [];
    const binding = new EngineCanvasBinding({
      service: () => ({ exclusive: true }) as unknown as EngineViewerService,
      composition: () => ({ w: 1920, h: 1080 }),
      quality: () => 1,
      present: ({ stage, before }) => {
        const p = new FakePresentation(stage, before);
        made.push(p);
        return p;
      }
    });
    const stage = document.createElement('div');
    const canvas = document.createElement('canvas');
    stage.append(canvas);
    document.body.append(stage);
    rect(stage, { left: 0, top: 0, width: 1000, height: 600 });
    rect(canvas, { left: 0, top: 0, width: 0, height: 0 });
    binding.bind(canvas);
    binding.resize();
    expect(made).toHaveLength(1);
    stage.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(made[0]!.disposed).toBe(true);
    expect(binding.bound).toBeNull();
  });

  it('does nothing with the engine off', () => {
    const { binding, canvas, disable } = setup();
    disable();
    expect(binding.bind(canvas)).toBe(false);
  });
});
