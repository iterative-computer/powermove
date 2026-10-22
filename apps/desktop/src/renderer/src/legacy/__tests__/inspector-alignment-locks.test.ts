// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';
import Harness from './inspector-alignment-lock-harness.svelte';
import { fakePowermoveAPI } from '../../../../extensions/timeline/fake-api.test-helper';

const mounted: Array<ReturnType<typeof mount>> = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function fixture(lock: 'layer' | 'nested group' | 'none' | 'all', protectedFirst = false) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 1000, h: 500, dur: 5 }); PM.time = 0;
  const editable = PM.mkLayer('shape', { name: 'Editable', p: { 'position.x': 200, 'position.y': 100 }, d: { w: 100, h: 100 } });
  const protectedLayer = PM.mkLayer('shape', { name: 'Protected', p: { 'position.x': 600, 'position.y': 100 }, d: { w: 100, h: 100 } });
  PM.proj.layers = [editable, protectedLayer]; PM.ProjectIndex.invalidate();
  const inner = PM.groupLayers([protectedLayer.id]);
  const outer = PM.groupLayers([inner.id]);
  if (lock === 'layer') protectedLayer.lock = true;
  if (lock === 'nested group' || lock === 'all') outer.lock = true;
  if (lock === 'all') editable.lock = true;
  const layers = protectedFirst ? [protectedLayer, editable] : [editable, protectedLayer];
  PM.selectLayers(layers.map(layer => layer.id)); PM.hist.clear();
  const harness = fakePowermoveAPI(vi);
  const { api } = harness;
  harness.state.project = PM.proj; harness.state.selection = PM.sel; harness.state.time = PM.time;
  Object.assign(api.anim, { ev: PM.ev, evP: PM.evP, findProp: PM.findProp, worldMatrix: PM.worldMatrix });
  Object.assign(api.model, { layer: PM.L });
  Object.assign(api.groups, { ancestors: PM.groupAncestors });
  Object.assign(api, { edit: PM.Edit, history: PM.hist });
  // Local shape bounds are exact; actual animation matrices supply world coordinates.
  vi.mocked(api.render.gl.bounds).mockReturnValue({ x0: -50, y0: -50, x1: 50, y1: 50 });
  const target = document.createElement('div'); document.body.append(target);
  flushSync(() => mounted.push(mount(Harness, { target, props: { api, layers } })));
  const alignRight = () => { target.querySelector<HTMLButtonElement>('[aria-label="Align right"]')!.click(); flushSync(); };
  const position = (id: string) => PM.worldMatrix(PM.L(id), PM.time).slice(4, 6);
  return { PM, editable, protectedLayer, outer, alignRight, position };
}

describe('inspector alignment locks', () => {
  for (const lock of ['layer', 'nested group'] as const) {
    it.each([false, true])('aligns editable siblings only through a ' + lock + ' lock (protected first: %s)', protectedFirst => {
      const { PM, editable, protectedLayer, position, alignRight } = fixture(lock, protectedFirst);
      const before = JSON.stringify(PM.proj.layers);
      const protectedBefore = position(protectedLayer.id);
      const editableY = position(editable.id)[1];
      alignRight();
      expect(position(editable.id)).toEqual([950, editableY]);
      expect(position(protectedLayer.id)).toEqual(protectedBefore);
      expect(PM.hist.undo()).toBe(true);
      expect(JSON.stringify(PM.proj.layers)).toBe(before);
      expect(PM.hist.undo()).toBe(false);
    });
  }

  it('makes no history entry when every selected layer is protected', () => {
    const { PM, alignRight } = fixture('all');
    const before = JSON.stringify(PM.proj.layers);
    alignRight();
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('aligns an unlocked selection together, preserving spacing and one-step Undo', () => {
    const { PM, editable, protectedLayer, position, alignRight } = fixture('none');
    const before = JSON.stringify(PM.proj.layers);
    const first = position(editable.id), second = position(protectedLayer.id);
    const delta = 950 - Math.max(first[0], second[0]);
    alignRight();
    expect(position(editable.id)).toEqual([first[0] + delta, first[1]]);
    expect(position(protectedLayer.id)).toEqual([second[0] + delta, second[1]]);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });
});
