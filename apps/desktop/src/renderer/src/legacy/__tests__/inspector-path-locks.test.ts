// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';
import { makeVectorPath, makeVertex } from '../core/vector-paths';
import Harness from './inspector-path-lock-harness.svelte';
import { fakePowermoveAPI } from '../../../../extensions/timeline/fake-api.test-helper';
import { channelBinding } from '../../controls/binding';
import Row from '../../controls/Row.svelte';
import NumField from '../../controls/NumField.svelte';
import SliderField from '../../controls/SliderField.svelte';
import SelectField from '../../controls/SelectField.svelte';
import ColorField from '../../controls/ColorField.svelte';
import Section from '../../controls/Section.svelte';

const mounted: Array<ReturnType<typeof mount>> = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function fixture(lock: 'layer' | 'nested group' | 'none', child = false) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 1;
  const primary = makeVectorPath(PM, 'Primary');
  primary.vertices = [makeVertex(PM, 0, 0), makeVertex(PM, 100, 0), makeVertex(PM, 50, 100)];
  primary.p.closed.v = true;
  const secondary = makeVectorPath(PM, 'Secondary');
  if (child) secondary.parent = primary.id;
  const layer = PM.mkLayer('shape', { name: 'Path target', d: { paths: [primary, secondary] } });
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
  Object.assign(api.model, { CH: PM.CH, layer: PM.L, P: PM.P, curComp: () => PM.proj });
  Object.assign(api.groups, { ancestors: PM.groupAncestors });
  Object.assign(api, { edit: PM.Edit, history: PM.hist });
  Object.assign(api.ui.controls, { Row, NumField, SliderField, SelectField, ColorField, Section,
    binding: { channelBinding: (id: string, path: string, options: any) => channelBinding(api, id, path, options) } });
  const target = document.createElement('div'); document.body.append(target);
  flushSync(() => mounted.push(mount(Harness, { target, props: { api, layer } })));
  const button = (label: string) => target.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
  const selector = [...target.querySelectorAll<HTMLSelectElement>('select')].find(select => select.closest('.row')?.querySelector('.k')?.textContent?.trim() === 'Group')!;
  const run = (action: 'Add' | 'Remove' | 'Group') => {
    if (action === 'Add') button('Add path').click();
    else if (action === 'Remove') button('Remove Primary').click();
    else { selector.value = '1'; selector.dispatchEvent(new Event('change', { bubbles: true })); }
    flushSync();
  };
  return { PM, layer, primary, secondary, outer, button, selector, run };
}

const actions = ['Add', 'Remove', 'Group'] as const;
describe('inspector shape path locks', () => {
  for (const lock of ['layer', 'nested group'] as const) {
    it.each(actions)('protects ' + lock + ' locks from %s actions', action => {
      const { PM, layer, run } = fixture(lock, action === 'Remove');
      const before = JSON.stringify(layer.d.paths);
      run(action);
      expect(JSON.stringify(PM.L(layer.id).d.paths)).toBe(before);
      expect(PM.hist.undo()).toBe(false);
    });
  }

  it.each(actions)('rechecks locks after controls rendered for %s', action => {
    const { PM, layer, outer, run } = fixture('none', action === 'Remove');
    const before = JSON.stringify(layer.d.paths);
    outer.lock = true;
    run(action);
    expect(JSON.stringify(PM.L(layer.id).d.paths)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('disables protected structural edits while keeping path inspection available', () => {
    const { PM, button, selector } = fixture('nested group');
    expect(button('Add path').disabled).toBe(true);
    expect(button('Remove Primary').disabled).toBe(true);
    expect(selector.closest('fieldset')?.disabled).toBe(true);
    expect(button('Edit Primary vertices').disabled).toBe(false);
    button('Edit Primary vertices').click(); flushSync();
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(actions)('keeps %s actions editable with one-step Undo', action => {
    const { PM, layer, primary, secondary, run } = fixture('none', action === 'Remove');
    const before = JSON.stringify(layer.d.paths);
    run(action);
    const paths = PM.L(layer.id).d.paths;
    if (action === 'Add') expect(paths).toHaveLength(3);
    else if (action === 'Remove') { expect(paths).toHaveLength(1); expect(paths[0].parent).toBe(null); }
    else expect(paths.find((path: any) => path.id === primary.id).parent).toBe(secondary.id);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).d.paths)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });
});
