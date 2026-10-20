// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';
import Harness from './inspector-type-axis-lock-harness.svelte';
import { fakePowermoveAPI } from '../../../../extensions/timeline/fake-api.test-helper';
import Row from '../../controls/Row.svelte';
import NumField from '../../controls/NumField.svelte';
import SliderField from '../../controls/SliderField.svelte';

const mounted: Array<ReturnType<typeof mount>> = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

const variants = [
  { name: 'a static axis', animated: false, time: 2, count: 1, value: 450 },
  { name: 'an animated axis between keys', animated: true, time: 2, count: 3, value: 500 },
  { name: 'an existing axis key', animated: true, time: 1, count: 1, value: 400 }
] as const;

function fixture(lock: 'layer' | 'nested group' | 'none', variant: typeof variants[number] = variants[1]) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = variant.time;
  const layer = PM.mkLayer('text', { name: 'Axis target', from: 1, d: { 'fontAxis.wght': PM.P(450) } });
  const property = layer.d['fontAxis.wght'];
  if (variant.animated) {
    PM.setKeyOn(property, 0, 400, 'linear', PM.proj.fps);
    PM.setKeyOn(property, 2, 600, 'linear', PM.proj.fps);
  }
  PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  const inner = PM.groupLayers([layer.id]);
  const outer = PM.groupLayers([inner.id]);
  if (lock === 'layer') layer.lock = true;
  if (lock === 'nested group') outer.lock = true;
  PM.selectLayers(layer.id); PM.hist.clear();
  const harness = fakePowermoveAPI(vi);
  const { api } = harness;
  harness.state.project = PM.proj; harness.state.selection = PM.sel; harness.state.time = PM.time;
  Object.assign(api.anim, { ev: PM.ev, evP: PM.evP, findProp: PM.findProp, hasKeyAt: PM.hasKeyAt,
    setKeyOn: PM.setKeyOn, removeKey: PM.removeKey, resolveContent: PM.resolveContent, touch: PM.touch });
  Object.assign(api.model, { layer: PM.L, P: PM.P, curComp: () => PM.proj });
  Object.assign(api.groups, { ancestors: PM.groupAncestors });
  Object.assign(api, { edit: PM.Edit, history: PM.hist });
  Object.assign(api.ui.controls, { Row, NumField, SliderField });
  const target = document.createElement('div'); document.body.append(target);
  flushSync(() => mounted.push(mount(Harness, { target, props: { api, layer } })));
  const button = target.querySelector<HTMLButtonElement>('.property-stopwatch')!;
  const input = target.querySelector<HTMLInputElement>('[role="spinbutton"]')!;
  const slider = target.querySelector<HTMLElement>('[role="slider"]')!;
  return { PM, layer, outer, button, input, slider, property };
}

describe('inspector variable font axis locks', () => {
  for (const lock of ['layer', 'nested group'] as const) {
    it.each(variants)('cannot toggle $name through a ' + lock + ' lock', variant => {
      const { PM, layer, button, input } = fixture(lock, variant);
      const before = JSON.stringify(layer.d['fontAxis.wght']);
      expect(Number(input.value)).toBe(variant.value);
      button.click(); flushSync();
      expect(JSON.stringify(PM.L(layer.id).d['fontAxis.wght'])).toBe(before);
      expect(button.disabled).toBe(true);
      expect(PM.hist.undo()).toBe(false);
    });
  }

  it('rechecks a nested lock changed after the axis control rendered', () => {
    const { PM, layer, outer, button } = fixture('none');
    const before = JSON.stringify(layer.d['fontAxis.wght']);
    outer.lock = true;
    button.click(); flushSync();
    expect(JSON.stringify(PM.L(layer.id).d['fontAxis.wght'])).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(variants)('still toggles $name on editable layers with one-step Undo', variant => {
    const { PM, layer, button, input } = fixture('none', variant);
    const before = JSON.stringify(layer.d['fontAxis.wght']);
    expect(Number(input.value)).toBe(variant.value);
    expect(button.disabled).toBe(false);
    button.click(); flushSync();
    expect(PM.L(layer.id).d['fontAxis.wght'].kf).toHaveLength(variant.count);
    expect(PM.evP(PM.L(layer.id), PM.L(layer.id).d['fontAxis.wght'], PM.time, 'c.fontAxis.wght')).toBe(variant.name === 'an existing axis key' ? 600 : variant.value);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).d['fontAxis.wght'])).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(['layer', 'nested group'] as const)('keeps numeric axis values protected through a %s lock', lock => {
    const { PM, layer, slider, input } = fixture(lock);
    const before = JSON.stringify(layer.d['fontAxis.wght']);
    expect(Number(input.value)).toBe(500);
    slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); flushSync();
    expect(JSON.stringify(PM.L(layer.id).d['fontAxis.wght'])).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('keeps numeric axis editing available and undoable on editable layers', () => {
    const { PM, layer, slider } = fixture('none');
    const before = JSON.stringify(layer.d['fontAxis.wght']);
    slider.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); flushSync();
    expect(PM.evP(PM.L(layer.id), PM.L(layer.id).d['fontAxis.wght'], PM.time, 'c.fontAxis.wght')).toBeCloseTo(500.01);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).d['fontAxis.wght'])).toBe(before);
  });
});
