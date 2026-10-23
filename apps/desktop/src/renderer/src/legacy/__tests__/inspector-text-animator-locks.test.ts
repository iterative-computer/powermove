// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';
import Harness from './inspector-text-animator-lock-harness.svelte';
import { fakePowermoveAPI } from '../../../../extensions/timeline/fake-api.test-helper';
import { ANIMATOR_PRESETS, makeAnimator } from '../../../../extensions/inspector/text-animators';
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

function fixture(lock: 'layer' | 'nested group' | 'none') {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 1;
  const layer = PM.mkLayer('text', { name: 'Animator target' });
  layer.d.animators = [makeAnimator(PM.P, 'entrance', 'Entrance', ANIMATOR_PRESETS[0]!, 64)];
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
  Object.assign(api.model, { CH: PM.CH, layer: PM.L, P: PM.P, curComp: () => PM.proj });
  Object.assign(api.groups, { ancestors: PM.groupAncestors });
  Object.assign(api, { edit: PM.Edit, history: PM.hist });
  Object.assign(api.ui.controls, { Row, NumField, SliderField, SelectField, ColorField, Section,
    binding: { channelBinding: (id: string, path: string, options: any) => channelBinding(api, id, path, options) } });
  const target = document.createElement('div'); document.body.append(target);
  flushSync(() => mounted.push(mount(Harness, { target, props: { api, layer } })));
  const button = (label: string) => target.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
  const menuAction = (buttonLabel: string, label: string) => {
    button(buttonLabel).dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true }));
    const items = vi.mocked(api.ui.menu).mock.calls.at(-1)![1] as any[];
    const item = items.find(item => item.label === label);
    expect(item, label).toBeDefined();
    return () => { if (!item.disabled) item.run(); flushSync(); };
  };
  const selector = [...target.querySelectorAll<HTMLSelectElement>('select')].find(select => select.closest('.row')?.querySelector('.k')?.textContent?.trim() === 'Based on')!;
  const run = (action: 'Add' | 'Remove' | 'Toggle' | 'Selector') => {
    if (action === 'Add') menuAction('Add text animator', 'Fade')();
    else if (action === 'Remove') menuAction('Entrance options', 'Remove')();
    else if (action === 'Toggle') { button('Disable Entrance').click(); flushSync(); }
    else { selector.value = '1'; selector.dispatchEvent(new Event('change', { bubbles: true })); flushSync(); }
  };
  return { PM, layer, outer, run, menuAction, selector, button };
}

const actions = ['Add', 'Remove', 'Toggle', 'Selector'] as const;
describe('inspector text animator locks', () => {
  for (const lock of ['layer', 'nested group'] as const) {
    it.each(actions)('protects ' + lock + ' locks from %s actions', action => {
      const { PM, layer, run } = fixture(lock);
      const before = JSON.stringify(layer.d.animators);
      run(action);
      expect(JSON.stringify(PM.L(layer.id).d.animators)).toBe(before);
      expect(PM.hist.undo()).toBe(false);
    });
  }

  it('rechecks a group lock after the preset menu opened', () => {
    const { PM, layer, outer, menuAction } = fixture('none');
    const before = JSON.stringify(layer.d.animators);
    const add = menuAction('Add text animator', 'Fade');
    outer.lock = true;
    add();
    expect(JSON.stringify(PM.L(layer.id).d.animators)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('rechecks a group lock after selector controls rendered', () => {
    const { PM, layer, outer, run } = fixture('none');
    const before = JSON.stringify(layer.d.animators);
    outer.lock = true;
    run('Selector');
    expect(JSON.stringify(PM.L(layer.id).d.animators)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('disables protected edits while keeping animator inspection available', () => {
    const { PM, button, selector } = fixture('nested group');
    expect(button('Add text animator').disabled).toBe(true);
    expect(button('Disable Entrance').disabled).toBe(true);
    expect(selector.closest('fieldset')?.disabled).toBe(true);
    expect(button('Collapse Entrance').disabled).toBe(false);
    button('Collapse Entrance').click(); flushSync();
    expect(button('Expand Entrance').disabled).toBe(false);
    button('Expand Entrance').click(); flushSync();
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(actions)('keeps %s actions editable with one-step Undo', action => {
    const { PM, layer, run } = fixture('none');
    const before = JSON.stringify(layer.d.animators);
    run(action);
    const animators = PM.L(layer.id).d.animators;
    if (action === 'Add') expect(animators).toHaveLength(2);
    else if (action === 'Remove') expect(animators).toHaveLength(0);
    else if (action === 'Toggle') expect(animators[0].enabled).toBe(false);
    else expect(animators[0].unit).toBe('words');
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).d.animators)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });
});
