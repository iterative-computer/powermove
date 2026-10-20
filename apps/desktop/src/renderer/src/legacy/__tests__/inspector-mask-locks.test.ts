// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';
import Harness from './inspector-mask-lock-harness.svelte';
import { fakePowermoveAPI } from '../../../../extensions/timeline/fake-api.test-helper';
import { channelBinding } from '../../controls/binding';
import Row from '../../controls/Row.svelte';
import NumField from '../../controls/NumField.svelte';
import SliderField from '../../controls/SliderField.svelte';
import ToggleField from '../../controls/ToggleField.svelte';
import SelectField from '../../controls/SelectField.svelte';
import Section from '../../controls/Section.svelte';

const mounted: Array<ReturnType<typeof mount>> = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function fixture(lock: 'layer' | 'nested group' | 'none') {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 1;
  const layer = PM.mkLayer('shape', { name: 'Mask target' });
  layer.masks = [PM.mkMask('rect', PM.proj)];
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
    setKeyOn: PM.setKeyOn, removeKey: PM.removeKey, touch: PM.touch });
  Object.assign(api.model, { CH: PM.CH, layer: PM.L, mkMask: PM.mkMask, MASK_SHAPES: PM.MASK_SHAPES, curComp: () => PM.proj });
  Object.assign(api.groups, { ancestors: PM.groupAncestors });
  Object.assign(api, { edit: PM.Edit, history: PM.hist });
  Object.assign(api.ui.controls, { Row, NumField, SliderField, ToggleField, SelectField, Section,
    binding: { channelBinding: (id: string, path: string, options: any) => channelBinding(api, id, path, options) } });
  const target = document.createElement('div'); document.body.append(target);
  flushSync(() => mounted.push(mount(Harness, { target, props: { api, layer } })));
  const button = (action: 'Add mask' | 'Delete mask 1') => target.querySelector<HTMLButtonElement>(`[aria-label="${action}"]`)!;
  return { PM, layer, outer, button };
}

describe('inspector mask locks', () => {
  it.each(['layer', 'nested group'] as const)('cannot add or remove a mask through a %s lock', lock => {
    const { PM, layer, button } = fixture(lock);
    const before = JSON.stringify(layer.masks);
    for (const action of ['Add mask', 'Delete mask 1'] as const) {
      button(action).click(); flushSync();
      expect(JSON.stringify(PM.L(layer.id).masks)).toBe(before);
      expect(button(action).disabled).toBe(true);
    }
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(['Add mask', 'Delete mask 1'] as const)('rechecks locks changed after the controls rendered for %s', action => {
    const { PM, layer, outer, button } = fixture('none');
    const before = JSON.stringify(layer.masks);
    outer.lock = true;
    button(action).click(); flushSync();
    expect(JSON.stringify(PM.L(layer.id).masks)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(['Add mask', 'Delete mask 1'] as const)('keeps %s available on editable layers with one-step Undo', action => {
    const { PM, layer, button } = fixture('none');
    const before = JSON.stringify(layer.masks);
    expect(button(action).disabled).toBe(false);
    button(action).click(); flushSync();
    expect(PM.L(layer.id).masks).toHaveLength(action === 'Add mask' ? 2 : 0);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).masks)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });
});
