// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import activateExtension from './index';
import { fakePowermoveAPI, type FakeAPIHarness } from './fake-api.test-helper';

const harnesses: FakeAPIHarness[] = [];
beforeEach(() => {
  vi.spyOn(window, 'requestAnimationFrame').mockReturnValue(1);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as never);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
});
afterEach(() => {
  harnesses.splice(0).forEach(harness => harness.dispose());
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function dialog(initialTime = 1) {
  const harness = fakePowermoveAPI(vi);
  harnesses.push(harness);
  const key = { i: 'edited', t: initialTime, v: 50 };
  const prop = { v: 100, expr: null, kf: [key, { i: 'neighbor', t: 2, v: 100 }].sort((a, b) => a.t - b.t) };
  const layer = { id: 'late-layer', name: 'Late layer', type: 'solid', from: 2, dur: 3, p: { opacity: prop } };
  harness.state.project.layers = [layer];
  harness.state.selection.layers = [layer.id];
  activateExtension(harness.api);
  const body = document.createElement('div');
  document.body.append(body);
  harness.state.panel!.build!(body, { spec: {} });
  const timeline = harness.state.timeline as any;
  timeline.rows = [{ kind: 'layer', L: layer }, { kind: 'prop', L: layer, key: 'opacity', prop }];
  timeline.scrollT = Math.min(0, layer.from + key.t - 1);
  const x = timeline.gut + (layer.from + key.t - timeline.scrollT) * timeline.pps;
  const y = timeline.ruler + timeline.row * 1.5;
  const event = new MouseEvent('dblclick', { clientX: x, clientY: y });
  Object.defineProperties(event, { offsetX: { value: x }, offsetY: { value: y } });
  body.querySelector('canvas')!.dispatchEvent(event);
  expect(harness.api.ui.modal).toHaveBeenCalledOnce();
  const modal = vi.mocked(harness.api.ui.modal).mock.calls[0]![0] as any;
  const time = modal.body.querySelector('[aria-label="Time (s)"]') as HTMLInputElement;
  const value = modal.body.querySelector('[aria-label="Value"]') as HTMLInputElement;
  const apply = modal.actions.find((action: any) => action.label === 'Apply').run;
  const status = () => modal.body.querySelector('[role="status"]').textContent;
  return { harness, layer, prop, key, time, value, apply, status };
}

describe('numeric keyframe time editing', () => {
  it.each([0, 1, 6, 10])('moves a key to composition time %ss outside its layer strip', time => {
    const fixture = dialog();
    fixture.time.value = String(time);
    fixture.value.value = '75';
    expect(fixture.apply()).not.toBe(false);
    expect(fixture.status()).toBe('');
    expect(fixture.key.t).toBe(time - fixture.layer.from);
    expect(fixture.key.v).toBe(75);
    expect(fixture.prop.kf.map(key => key.t)).toEqual([time - 2, 2].sort((a, b) => a - b));
    expect([fixture.layer.from, fixture.layer.dur]).toEqual([2, 3]);
    expect(fixture.harness.api.history.do).toHaveBeenCalledOnce();
  });

  it.each([-1 / 30, 10 + 1 / 30])('rejects a key outside the composition at %ss without changing it', time => {
    const fixture = dialog();
    fixture.time.value = String(time);
    fixture.value.value = '75';
    expect(fixture.apply()).toBe(false);
    expect(fixture.status()).toContain('outside the composition');
    expect([fixture.key.t, fixture.key.v]).toEqual([1, 50]);
    expect(fixture.harness.api.history.do).not.toHaveBeenCalled();
  });

  it('still rejects collisions on the neighboring keyframe', () => {
    const fixture = dialog();
    fixture.time.value = '4';
    fixture.value.value = '75';
    expect(fixture.apply()).toBe(false);
    expect(fixture.status()).toContain('already sits on that frame');
    expect([fixture.key.t, fixture.key.v]).toEqual([1, 50]);
  });

  it('snaps composition times to the nearest frame', () => {
    const fixture = dialog();
    fixture.time.value = '1.01';
    expect(fixture.apply()).not.toBe(false);
    expect(fixture.key.t).toBe(-1);
  });

  it.each([-3, 9])('retains an existing out-of-composition key at local time %ss while editing its value', time => {
    const fixture = dialog(time);
    fixture.value.value = '75';
    expect(fixture.apply()).not.toBe(false);
    expect([fixture.key.t, fixture.key.v]).toEqual([time, 75]);
    // As with a drag, the old time does not permit extending farther out.
    fixture.time.value = String(fixture.layer.from + time + (time < 0 ? -1 : 1));
    expect(fixture.apply()).toBe(false);
    expect([fixture.key.t, fixture.key.v]).toEqual([time, 75]);
  });
});
