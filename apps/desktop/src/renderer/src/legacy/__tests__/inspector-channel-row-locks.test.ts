// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';
import Harness from './inspector-channel-row-lock-harness.svelte';
import { fakePowermoveAPI } from '../../../../extensions/timeline/fake-api.test-helper';
import { channelBinding } from '../../controls/binding';
import Row from '../../controls/Row.svelte';
import NumField from '../../controls/NumField.svelte';
import SliderField from '../../controls/SliderField.svelte';
import type { ControlsAPI } from '../../kernel/api';

const mounted: Array<ReturnType<typeof mount>> = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function fixture(lock: 'layer' | 'nested group' | 'none') {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 1;
  const layer = PM.mkLayer('shape', { name: 'Inspector menu target' });
  PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  PM.setKey(layer, 'opacity', 0, 20); PM.setKey(layer, 'opacity', 2, 80);
  const inner = PM.groupLayers([layer.id]);
  const outer = PM.groupLayers([inner.id]);
  if (lock === 'layer') layer.lock = true;
  if (lock === 'nested group') outer.lock = true;
  PM.selectLayers(layer.id); PM.hist.clear();
  const harness = fakePowermoveAPI(vi);
  const { api } = harness;
  harness.state.project = PM.proj;
  harness.state.selection = PM.sel;
  harness.state.time = PM.time;
  Object.assign(api.anim, {
    ev: PM.ev, evP: PM.evP, findProp: PM.findProp, hasKeyAt: PM.hasKeyAt,
    setKeyOn: PM.setKeyOn, removeKey: PM.removeKey, applyEaseTo: PM.applyEaseTo, touch: PM.touch,
  });
  Object.assign(api.model, { CH: PM.CH, layer: PM.L, curComp: () => PM.proj });
  Object.assign(api.groups, { ancestors: PM.groupAncestors });
  Object.assign(api, { edit: PM.Edit, history: PM.hist });
  Object.assign(api.ui.controls, {
    Row: Row as ControlsAPI['Row'], NumField: NumField as ControlsAPI['NumField'],
    SliderField: SliderField as ControlsAPI['SliderField'],
    binding: { channelBinding: (id: string, path: string, options: any) => channelBinding(api, id, path, options) },
  });
  const target = document.createElement('div'); document.body.append(target);
  flushSync(() => mounted.push(mount(Harness, { target, props: { api, layer } })));
  const row = target.querySelector('[data-channel="opacity"]')!;
  row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
  const items = vi.mocked(api.ui.menu).mock.calls.at(-1)![1] as any[];
  const choose = (label: string) => {
    const item = items.find(item => item.label === label);
    expect(item, label).toBeDefined();
    if (!item.disabled) item.run();
  };
  return { PM, layer, outer, choose, items };
}

describe('inspector context menu locks', () => {
  it.each(['layer', 'nested group'] as const)('cannot add a key through a %s lock', lock => {
    const { layer, choose } = fixture(lock);
    const before = JSON.stringify(layer.p.opacity);
    choose('Add Keyframe at Playhead');
    expect(JSON.stringify(layer.p.opacity)).toBe(before);
  });

  it.each(['layer', 'nested group'] as const)('cannot replace easing through a %s lock', lock => {
    const { layer, choose } = fixture(lock);
    const before = JSON.stringify(layer.p.opacity);
    choose('Power');
    expect(JSON.stringify(layer.p.opacity)).toBe(before);
  });

  it.each(['Add Keyframe at Playhead', 'Power'])('rechecks a group lock changed after the menu opened for %s', action => {
    const { layer, outer, choose } = fixture('none');
    const before = JSON.stringify(layer.p.opacity);
    outer.lock = true;
    choose(action);
    expect(JSON.stringify(layer.p.opacity)).toBe(before);
  });

  it('disables editing choices for protected layers while keeping graph inspection available', () => {
    const { items } = fixture('nested group');
    for (const item of items.filter(item => item.run && item.label !== 'Show in Graph Editor')) {
      expect(item.disabled, item.label).toBe(true);
    }
    expect(items.find(item => item.label === 'Show in Graph Editor').disabled).not.toBe(true);
  });

  it('still adds a key to an unlocked layer and supports one-step Undo', () => {
    const { PM, layer, choose } = fixture('none');
    choose('Add Keyframe at Playhead');
    expect(layer.p.opacity.kf).toHaveLength(3);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(layer.id).p.opacity.kf).toHaveLength(2);
  });

  it('still changes easing on an unlocked layer and supports one-step Undo', () => {
    const { PM, layer, choose } = fixture('none');
    const before = JSON.stringify(layer.p.opacity);
    choose('Power');
    expect(JSON.stringify(layer.p.opacity)).not.toBe(before);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).p.opacity)).toBe(before);
  });
});
