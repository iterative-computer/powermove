import { describe, expect, it } from 'vitest';
import { installLayerGroups } from './layer-groups';
import { makePM } from '../__tests__/make-pm';

function normalize(layers: any[]) {
  const PM: any = { proj: { layers } };
  installLayerGroups(PM); PM.normalizeGroupStack(); return PM.proj.layers;
}

// Reference behavior from the previous implementation, including orphan repair
// and ID-based visitation. The real installed implementation is exercised above.
function previousNormalize(layers: any[]) {
  const ordered: any[] = [], seen = new Set<string>();
  const visit = (parent: string | null) => {
    for (const layer of layers) {
      if ((layer.group || null) !== parent || seen.has(layer.id)) continue;
      seen.add(layer.id); ordered.push(layer);
      if (layer.type === 'group') visit(layer.id);
    }
  };
  visit(null);
  for (const layer of layers) if (!seen.has(layer.id)) { layer.group = null; ordered.push(layer); }
  return ordered;
}

describe('group stack normalization', () => {
  it('reads memberships once instead of rescanning every layer for every group', () => {
    let reads = 0;
    const groups = Array.from({ length: 200 }, (_, i) => ({ id: `g${i}`, type: 'group', group: null }));
    const children = Array.from({ length: 800 }, (_, i) => {
      const group = `g${i % groups.length}`;
      return { id: `l${i}`, type: 'shape', get group() { reads++; return group; } };
    });
    const ordered = normalize([...groups, ...children]);
    expect(ordered).toHaveLength(1000);
    expect(ordered.slice(0, 5).map((layer: any) => layer.id)).toEqual(['g0', 'l0', 'l200', 'l400', 'l600']);
    expect(reads).toBe(800);
  });

  it('matches previous order and repairs across seeded malformed and nested inputs', () => {
    let seed = 892371;
    const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 2 ** 32; };
    for (let fixture = 0; fixture < 240; fixture++) {
      const count = 10 + Math.floor(random() * 70);
      const input = Array.from({ length: count }, (_, i) => ({
        id: random() < .08 ? 'duplicate' : `id${i}`,
        type: random() < .35 ? 'group' : 'shape',
        group: [null, undefined, '', 'missing', `id${Math.floor(random() * count)}`, 'duplicate'][Math.floor(random() * 6)],
        originalIndex: i,
      }));
      const reference = previousNormalize(input.map(layer => ({ ...layer })));
      const actual = normalize(input.map(layer => ({ ...layer })));
      expect(actual, `fixture ${fixture}`).toEqual(reference);
    }
  });

  it('preserves falsy group values and unusual duplicate IDs', () => {
    const input = [
      { id: 'g', type: 'group', group: undefined },
      { id: 'a', type: 'shape', group: 0 }, { id: 'b', type: 'shape', group: false },
      { id: 'c', type: 'shape', group: NaN }, { id: null, type: 'group', group: 'g' },
      { id: undefined, type: 'group', group: null }, { id: NaN, type: 'group', group: 'g' },
      { id: 'duplicate', type: 'shape', group: 'missing' },
      { id: 'duplicate', type: 'group', group: 'missing' },
      { id: 'orphan', type: 'shape', group: 'duplicate' },
    ];
    expect(normalize(input.map(layer => ({ ...layer })))).toEqual(previousNormalize(input.map(layer => ({ ...layer }))));
  });

  it('continues rejecting sparse layer arrays without publishing a partial stack', () => {
    const layers = new Array(3); layers[0] = { id: 'group', type: 'group' }; layers[2] = { id: 'child', type: 'shape', group: 'group' };
    const PM: any = { proj: { layers } }; installLayerGroups(PM);
    expect(() => PM.normalizeGroupStack()).toThrow(TypeError);
    expect(PM.proj.layers).toBe(layers);
  });

  it('preserves real grouping, moving, undo/redo and ungrouping order and world poses', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ dur: 5 }); PM.time = 1;
    const layers = Array.from({ length: 6 }, (_, i) => PM.mkLayer('shape', { name: `Layer ${i}` }));
    PM.proj.layers = [...layers]; PM.ProjectIndex.invalidate();
    const pose = PM.worldMatrix(layers[4], PM.time);
    expect(PM.Edit.apply({ type: 'group_layers', targets: [layers[1].id, layers[4].id], name: 'Inner' }).ok).toBe(true);
    const inner = PM.firstSel();
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual([layers[0].id, inner.id, layers[1].id, layers[4].id, layers[2].id, layers[3].id, layers[5].id]);
    expect(PM.worldMatrix(layers[4], PM.time)).toEqual(pose);
    const grouped = PM.proj.layers.map((layer: any) => layer.id);
    expect(PM.hist.undo()).toBe(true); expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual(layers.map(layer => layer.id));
    expect(PM.hist.redo()).toBe(true); expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual(grouped);
    expect(PM.Edit.apply({ type: 'move_to_group', targets: [layers[2].id], group: inner.id }).ok).toBe(true);
    expect(PM.Edit.apply({ type: 'ungroup_layers', targets: [inner.id] }).ok).toBe(true);
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual([layers[0].id, layers[1].id, layers[4].id, layers[2].id, layers[3].id, layers[5].id]);
    expect(PM.worldMatrix(PM.L(layers[4].id), PM.time)).toEqual(pose);
  });
});
