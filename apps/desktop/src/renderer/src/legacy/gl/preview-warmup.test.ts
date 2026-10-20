import { describe, expect, it, vi } from 'vitest';
import { createPreviewWarmup } from './preview-warmup';
import { makePM } from '../__tests__/make-pm';

function fixture() {
  const project = { layers: Array.from({ length: 20 }, (_, i) => ({ id: String(i), type: 'shape', from: 4, dur: 5, on: true })) };
  const state = { key: '1', project, time: 2.5, blocked: false };
  let time = 0;
  const callbacks: Array<(deadline: { timeRemaining(): number }) => void> = [];
  const prepare = vi.fn(() => { time++; });
  const request = createPreviewWarmup(() => state, prepare, callback => callbacks.push(callback), () => time);
  return { state, callbacks, prepare, request, run: () => callbacks.shift()!({ timeRemaining: () => 10 }) };
}

function opacityFixture() {
  const PM = makePM('core/easing', 'core/model', 'core/anim');
  PM.proj = PM.mkProject({ dur: 20, fps: 30 });
  PM.time = 0;
  const state = { key: '1', project: PM.proj, time: 2.5, blocked: false };
  let now = 0;
  const callbacks: Array<(deadline: { timeRemaining(): number }) => void> = [];
  const prepare = vi.fn(() => { now++; });
  const visible = vi.fn((layer: any, time: number) => PM.active(layer, time) && PM.worldOpacity(layer, time) > .001);
  const request = createPreviewWarmup(() => state, prepare, callback => callbacks.push(callback), () => now, visible);
  const layer = (type = 'text', keys?: Array<{ t: number; v: number; hold?: boolean }>) => {
    const result = PM.mkLayer(type); result.from = 0; result.dur = 20;
    if (keys) {
      result.p.opacity.kf = keys.map(key => ({ ...PM.KF(key.t, key.v, 'linear'), hold: key.hold === true }));
    }
    PM.proj.layers.push(result); PM.touch(); return result;
  };
  const run = () => callbacks.shift()!({ timeRemaining: () => 10 });
  const drain = () => { let count = 0; while (callbacks.length && count++ < 100) run(); expect(count).toBeLessThan(100); };
  return { PM, state, prepare, visible, callbacks, request, layer, run, drain };
}

describe('preview source warmup', () => {
  it('prepares visible children at an upcoming held ancestor scale cut', () => {
    const f = opacityFixture();
    const parent = f.layer('null');
    parent.p['scale.x'].kf = [{ ...f.PM.KF(0, 150, 'linear'), outInterp: 'hold' }, f.PM.KF(4, 100, 'linear')];
    const group = f.layer('group'); group.parent = parent.id;
    const child = f.layer('shape'); child.group = group.id;
    f.request(); f.drain();
    expect(f.prepare).toHaveBeenCalledOnce();
    expect(f.prepare).toHaveBeenCalledWith(child, 4);
    f.state.time = 3; f.request(); f.drain();
    expect(f.prepare).toHaveBeenCalledOnce();
  });
  it('warms a text opacity entrance even when the layer starts at zero', () => {
    const f = opacityFixture();
    const text = f.layer('text', [{ t: 4, v: 0 }, { t: 4.5, v: 100 }]);
    f.layer('shape');
    f.request(); f.drain();
    expect(f.prepare).toHaveBeenCalledOnce();
    expect(f.prepare).toHaveBeenCalledWith(text, 4 + 1 / 30);
    expect(f.PM.time).toBe(0);
    expect(f.PM.worldOpacity(text, 2.5)).toBe(0);
  });

  it('intersects nested group entrances and does not warm distant hidden scenes', () => {
    const f = opacityFixture();
    const outer = f.layer('group', [{ t: 4, v: 0 }, { t: 4.5, v: 100 }]);
    const inner = f.layer('group', [{ t: 3, v: 0 }, { t: 3.5, v: 100 }]); inner.group = outer.id;
    const child = f.layer('text'); child.group = inner.id;
    const distant = f.layer('group', [{ t: 12, v: 0 }, { t: 12.5, v: 100 }]);
    for (let i = 0; i < 100; i++) f.layer('text').group = distant.id;
    f.request(); f.drain();
    expect(f.prepare).toHaveBeenCalledOnce();
    expect(f.prepare).toHaveBeenCalledWith(child, 4 + 1 / 30);
    expect(f.visible.mock.calls.length).toBeLessThan(10);
    f.state.time = 10.5; f.request();
    f.run(); expect(f.prepare).toHaveBeenCalledTimes(3);
    f.drain(); expect(f.prepare).toHaveBeenCalledTimes(101);
  });

  it('respects hold keys, disabled groups and opacity expressions', () => {
    const f = opacityFixture();
    const held = f.layer('text', [{ t: 0, v: 0, hold: true }, { t: 4, v: 100 }]);
    const disabled = f.layer('group'); disabled.on = false;
    f.layer('text', [{ t: 4, v: 0 }, { t: 4.5, v: 100 }]).group = disabled.id;
    const expression = f.layer('text'); expression.p.opacity.expr = 'T > 4 ? 100 : 0';
    f.request(); f.drain();
    expect(f.prepare).toHaveBeenCalledOnce();
    expect(f.prepare).toHaveBeenCalledWith(held, 4);
  });

  it('does not gate child warmup on an ordinary parent opacity', () => {
    const f = opacityFixture();
    const parent = f.layer('shape'); parent.p.opacity.v = 0;
    const child = f.layer('text', [{ t: 4, v: 0 }, { t: 4.5, v: 100 }]); child.parent = parent.id;
    f.request(); f.drain();
    expect(f.prepare).toHaveBeenCalledWith(child, 4 + 1 / 30);
  });

  it('checks the idle time limit between visibility probes as well as between raster preparations', () => {
    let now = 0;
    const state = { key: '1', time: 0, blocked: false,
      project: { fps: 30, layers: [{ type: 'text', from: .1, dur: 3, p: { opacity: { v: 100, kf: [] } } }] } };
    const callbacks: Array<(deadline: { timeRemaining(): number }) => void> = [];
    const prepare = vi.fn(), visible = vi.fn(() => { now++; return false; });
    const request = createPreviewWarmup(() => state, prepare, callback => callbacks.push(callback), () => now, visible);
    request(); callbacks.shift()!({ timeRemaining: () => 10 });
    expect(visible).toHaveBeenCalledTimes(2);
    expect(prepare).not.toHaveBeenCalled();
    expect(callbacks).toHaveLength(1);
  });

  it('splits a simultaneous entrance into bounded idle slices without preparing current layers', () => {
    const f = fixture();
    f.state.project.layers.push({ id: 'current', type: 'shape', from: 0, dur: 8, on: true });
    f.request(); f.request(); expect(f.callbacks).toHaveLength(1);
    f.run(); expect(f.prepare).toHaveBeenCalledTimes(2);
    while (f.callbacks.length) f.run();
    expect(f.prepare).toHaveBeenCalledTimes(20);
    expect(f.prepare.mock.calls.every(([layer, time]: any) => layer.id !== 'current' && time === 4)).toBe(true);
    f.request(); expect(f.callbacks).toHaveLength(0);
  });

  it.each(['export', 'edit', 'project'] as const)('discards a queued slice after %s changes ownership', mode => {
    const f = fixture(); f.request();
    if (mode === 'export') f.state.blocked = true;
    if (mode === 'edit') f.state.key = '2';
    if (mode === 'project') f.state.project = { layers: [] };
    f.run(); expect(f.prepare).not.toHaveBeenCalled();
  });

  it('reschedules a replaced queue even when the paused redraw happened while a slice was pending', () => {
    const f = fixture();
    f.request();
    f.state.key = 'new-font-or-preview-size';
    f.request();
    f.run();
    expect(f.prepare).not.toHaveBeenCalled();
    expect(f.callbacks).toHaveLength(1);
    while (f.callbacks.length) f.run();
    expect(f.prepare).toHaveBeenCalledTimes(20);
  });

  it('does not force work into an exhausted idle deadline', () => {
    const f = fixture(); f.request();
    f.callbacks.shift()!({ timeRemaining: () => 0 });
    expect(f.prepare).not.toHaveBeenCalled();
    expect(f.callbacks).toHaveLength(1);
  });

  it('lets the normal render own layers reached before an idle callback runs', () => {
    const f = fixture(); f.request(); f.state.time = 4; f.run();
    expect(f.prepare).not.toHaveBeenCalled();
  });
});
