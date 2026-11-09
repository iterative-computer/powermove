import { describe, expect, it, vi } from 'vitest';
import { describeMap, filterGroups, materialEntries, materialUsage, sourceGroups, statusNote, usageTitle, installProperty, resetMaterial } from './materials';

function harness() {
  const model = { id: 'model', type: 'extension', name: 'Phone', from: 1, d: { definition: 'powermove.3d.object', data: { object: {
    source: { assetId: 'glb' }, material: { maps: {} }, sourceMaterials: { 0: { color: { layerId: 'paint' } } }, sourceMaterialProperties: { 0: { opacity: { v: 50, kf: [], expr: null } } } } } } };
  const layers: any[] = [
    model,
    { id: 'artwork', type: 'group', name: 'Screen Artwork', on: false },
    { id: 'paint', type: 'solid', name: 'Red Solid', on: false },
    { id: 'anim', type: 'precomp', name: 'Screen Animation', on: true },
    { id: 'loop', type: 'solid', name: 'Blurred by Phone', on: true },
    { id: 'clip', type: 'video', name: 'Clip', on: true },
    { id: 'cam', type: 'extension', name: 'Camera', d: { definition: 'powermove.3d.camera' } },
    { id: 'music', type: 'audio', name: 'Music' },
  ];
  const project = { id: 'p', compId: 'main', compName: 'Main', layers, comps: { inner: { id: 'inner', name: 'Inner' } }, assets: { img: { kind: 'image', name: 'Swatch.png' }, vid: { kind: 'video', name: 'Clip.mp4' } } };
  const api: any = {
    model: { layer: (id: string) => layers.find(layer => layer.id === id) },
    project: { get: () => project },
    scene3d: {
      listMaterials: () => [{ index: 0, name: 'Screen', slots: [], maps: {} }, { index: 1, name: 'Body', slots: [], maps: {} }],
      listEligibleSourceLayers: () => [{ group: 'Groups', layers: [{ id: 'artwork' }] },{ group: 'Comps', layers: [{ id: 'anim' }] }, { group: 'Solids', layers: [{ id: 'paint' }] }, { group: 'Footage', layers: [{ id: 'clip' }] }],
      listValidCompositions: () => [{ id: 'inner' }],
      materialLinks: () => [{ materialIndex: 0, slot: 'color', layerId: 'paint', status: 'ready' }],
      setMaterialMap: vi.fn(() => ({ ok: true })),
      setMaterialProperties: vi.fn(() => ({ ok: true })),
    },
    edit: { begin: vi.fn(), commit: vi.fn(), cancel: vi.fn() },
    ui: { toast: vi.fn() },
  };
  return { api, layers, project };
}

describe('Layers as Materials model', () => {
  it('lists Whole Model then each named material, with their own maps', () => {
    const { api } = harness();
    expect(materialEntries(api, 'model').map(entry => [entry.index, entry.name, entry.maps])).toEqual([
      [null, 'Whole Model', {}], [0, 'Screen', { color: { layerId: 'paint' } }], [1, 'Body', {}],
    ]);
  });

  it('groups sources and explains the ones that cannot be used', () => {
    const { api } = harness();
    const groups = sourceGroups(api, 'model', 'color');
    const option = (id: string) => groups.flatMap(group => group.options).find(item => item.id === id)!;
    expect(groups.map(group => group.label)).toEqual(['', 'Pre-comp Layers', 'Groups', 'Solid Layers', 'Footage Layers', 'Text & Shape Layers', 'Other Layers', 'Compositions', 'Images']);
    expect(option('layer:artwork')).toMatchObject({disabled:false,hidden:true,icon:'layers',map:{layerId:'artwork'}});
    expect(option('layer:paint')).toMatchObject({ disabled: false, hidden: true, map: { layerId: 'paint' } });
    expect(option('layer:loop')).toMatchObject({ disabled: true, detail: 'Uses this model, so it would loop' });
    expect(option('layer:cam')).toMatchObject({ disabled: true, detail: '3D layers can’t be painted onto a model' });
    expect(option('layer:music')).toMatchObject({ disabled: true, detail: 'Audio has no picture' });
    expect(option('comp:main')).toMatchObject({ disabled: true, detail: 'This model is in this comp, so it would loop' });
    expect(option('comp:inner')).toMatchObject({ disabled: false, map: { compositionId: 'inner' } });
    expect(option('image:img')).toMatchObject({ map: 'img' });
    expect(groups.flatMap(group => group.options).some(item => item.id === 'image:vid')).toBe(false);
  });

  it('filters by Comps / Footage and searches by name, keeping Original', () => {
    const { api } = harness();
    const groups = sourceGroups(api, 'model', 'color');
    expect(filterGroups(groups, 'comps', '').map(group => group.id)).toEqual(['original', 'precomps', 'compositions']);
    expect(filterGroups(groups, 'footage', '').map(group => group.id)).toEqual(['original', 'footage', 'images']);
    expect(filterGroups(groups, 'all', 'red').flatMap(group => group.options.map(option => option.id))).toEqual(['layer:paint', 'layer:loop']);
  });

  it('offers only images (and Original) on image-only slots', () => {
    const { api } = harness();
    expect(sourceGroups(api, 'model', 'normal').map(group => group.id)).toEqual(['original', 'images']);
  });

  it('describes current sources and missing links plainly', () => {
    const { api } = harness();
    expect(describeMap(api, 'model', 0, 'color', { layerId: 'paint' })).toMatchObject({ label: 'Red Solid', icon: 'solid', status: 'ready' });
    expect(describeMap(api, 'model', 0, 'emissive', null)).toMatchObject({ label: 'Original', status: 'original' });
    expect(describeMap(api, 'model', 0, 'color', { compositionId: 'inner' })).toMatchObject({ label: 'Inner', icon: 'stack' });
    expect(describeMap(api, 'model', 0, 'color', { layerId: 'gone' })).toMatchObject({ label: 'Missing layer', status: 'missing' });
    expect(statusNote('missing')).toMatch(/original material/);
    expect(statusNote('ready')).toBeNull();
  });

  it('reports where a source is used, mentioning hidden layers', () => {
    const { api } = harness();
    const uses = materialUsage(api, 'paint');
    expect(uses).toEqual([{ layerId: 'model', name: 'Phone', uses: ['Screen · Base Color'] }]);
    expect(usageTitle(uses, true)).toBe('Used as a material — on Phone (Screen · Base Color). It’s hidden, which is fine: hidden layers still work as materials. Click to select Phone.');
    expect(materialUsage(api, 'anim')).toEqual([]);
  });

  it('installs a property with a key at layer time, and resets a material in one step', () => {
    const { api } = harness();
    installProperty(api, 'model', 0, 'roughness', .3, 2.5);
    expect(api.scene3d.setMaterialProperties).toHaveBeenCalledWith('model', 0, { roughness: { v: .3, kf: [{ t: 1.5, v: .3 }], expr: null } }, expect.objectContaining({ label: 'Add keyframe for Roughness' }));
    expect(resetMaterial(api, 'model', 0)).toBe(true);
    expect(api.scene3d.setMaterialMap).toHaveBeenCalledWith('model', 0, 'color', null, expect.anything());
    expect(api.scene3d.setMaterialProperties).toHaveBeenLastCalledWith('model', 0, { opacity: null }, expect.anything());
    expect(api.edit.begin).toHaveBeenCalledTimes(1);
    expect(api.edit.commit).toHaveBeenCalledTimes(1);
    expect(resetMaterial(api, 'model', 1)).toBe(false);
  });
});
