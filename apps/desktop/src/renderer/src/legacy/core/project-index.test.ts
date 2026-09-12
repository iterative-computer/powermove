import { describe, expect, it } from 'vitest';

import { makePM } from '../__tests__/make-pm';

function indexedProject() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim');
  PM.proj = PM.mkProject({ name: 'Index fixture', dur: 10 });
  const root = PM.mkLayer('solid', { name: 'Root' });
  root.id = 'root';
  const child = PM.mkLayer('solid', { name: 'Child' });
  child.id = 'child';
  child.parent = root.id;
  const grandchild = PM.mkLayer('solid', { name: 'Grandchild' });
  grandchild.id = 'grandchild';
  grandchild.parent = child.id;
  PM.proj.layers = [root, child, grandchild];
  return { PM, root, child, grandchild };
}

describe('project index', () => {
  it('resolves ids, exact names, selection, and descendants without serialized project copies', () => {
    const { PM, root, child } = indexedProject();
    expect(PM.L('child')).toBe(child);
    expect(PM.byName('ROOT')).toBe(root);
    PM.sel.layers = ['child', 'missing'];
    expect(PM.selLayers()).toEqual([child]);
    expect(PM.ProjectIndex.parentOptions(root).map((option: any) => option.v)).toEqual([]);
    expect(PM.ProjectIndex.parentOptions(child).map((option: any) => option.v)).toEqual(['root']);
  });

  it('rebuilds after structure changes and indexes selected keyframes by identity', () => {
    const { PM, child } = indexedProject();
    const key = PM.setKeyOn(child.p.opacity, 1, 50, 'linear', 30);
    PM.ProjectIndex.invalidateKeyframes();
    expect(PM.ProjectIndex.keyframe(key.i)).toBe(key);
    expect(PM.ProjectIndex.layerForKeyframe(key.i)).toBe(child);

    const added = PM.mkLayer('solid', { name: 'Added' });
    added.id = 'added';
    PM.addLayer(added, 0);
    expect(PM.L('added')).toBe(added);
  });

  it('uses its sorted time index to return only active layers', () => {
    const { PM, root, child, grandchild } = indexedProject();
    root.from = 0; root.dur = 2;
    child.from = 5; child.dur = 3;
    grandchild.from = 7; grandchild.dur = 3;
    PM.ProjectIndex.invalidate();
    expect(PM.ProjectIndex.activeAt(1)).toEqual([root]);
    expect(PM.ProjectIndex.activeAt(7.5)).toEqual([child, grandchild]);
  });
});

it('indexes content keys and refreshes newly added keys after an earlier lookup', () => {
  const { PM, child } = indexedProject();
  child.d.color = PM.P('#ff0000');
  const color = PM.setKeyOn(child.d.color, 1, '#0000ff');
  PM.sel.keys = [color.i];
  expect(PM.resolveSelectedKeys()).toEqual([color]);
  expect(PM.ProjectIndex.layerForKeyframe(color.i)).toBe(child);
  const position = PM.setKeyOn(child.p['position.y'], 2, 200);
  expect(PM.ProjectIndex.keyframe(position.i)).toBe(position);
  const saved = JSON.parse(JSON.stringify(PM.proj));
  PM.replaceProject(saved, { selection: { layers: [child.id], keys: [color.i], chan: 'c.color' } });
  expect(PM.sel.keys).toEqual([color.i]);
  expect(PM.resolveSelectedKeys()[0].i).toBe(color.i);
  PM.removeKey(PM.proj.layers[1].p['position.y'], PM.ProjectIndex.keyframe(position.i));
  expect(PM.ProjectIndex.keyframe(position.i)).toBeNull();
});
