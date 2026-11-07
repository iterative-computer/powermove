import { describe, expect, it, vi } from 'vitest';
import { hasMaterialRows, isMaterialPropKey, materialSectionRows, materialSourceMap } from './material-rows';
import { propertySection } from './property-sections';

const model = (maps: any = {}, sourceMaterials: any = {}, sourceMaterialProperties: any = {}) => ({
  id: 'model', type: 'extension', name: 'Phone', from: 0,
  d: { definition: 'powermove.3d.object', data: { object: { source: { assetId: 'a' }, material: { maps }, sourceMaterials, sourceMaterialProperties } } },
});
const properties = [
  { key: 'opacity', label: 'Opacity', kind: 'num' },
  { key: 'roughness', label: 'Roughness', kind: 'num' },
];
function apiFor(layers: any[], installed: Record<string, any> = {}) {
  const service = {
    entries: vi.fn(() => [{ index: null, name: 'Whole Model', maps: {} }, { index: 0, name: 'Screen', maps: { color: { layerId: 'paint' } } }]),
    properties,
    propertyPath: (index: number | null, key: string) => `mi.${index == null ? '' : `${index}.`}${key}`,
    propertyValues: vi.fn(() => ({ opacity: 100, roughness: .4 })),
  };
  return {
    service,
    api: {
      services: { get: (name: string) => name === 'materials' ? service : null },
      anim: { findProp: (_layer: any, path: string) => installed[path] ?? null },
      transport: { time: () => 0 },
      project: { get: () => ({ layers }), revision: () => 1 },
    } as any,
  };
}

describe('timeline Materials group', () => {
  it('lists each material, and opened ones show Emissive and every Material Option', () => {
    const layer = model();
    const keyed = { v: .2, kf: [{ t: 0, v: .2 }, { t: 1, v: .8 }], expr: null };
    const { api } = apiFor([layer], { 'mi.roughness': keyed });
    const rows = materialSectionRows(api, layer, (_l, index) => index === 0);
    expect(rows.map(row => `${row.kind}:${row.key}${row.virtual ? '*' : ''}`)).toEqual([
      'heading:section:materials',
      'material:material:all:color', 'prop:mi.roughness',
      'material:material:0:color', 'material:material:0:emissive', 'prop:mi.0.opacity*', 'prop:mi.0.roughness*',
    ]);
    // A closed material still shows its animated options; stand-ins carry the inherited value.
    expect(rows[2].prop).toBe(keyed);
    const screenRoughness = 'mi.0.roughness';
    expect(rows.find(row => row.key === screenRoughness).prop).toEqual({ v: .4, kf: [], expr: null });
    expect(rows.find(row => row.key === 'material:0:color')).toMatchObject({ label: 'Screen', entry: true, open: true });
  });

  it('only models with the materials service get the group', () => {
    const { api } = apiFor([]);
    expect(hasMaterialRows(api, model())).toBe(true);
    expect(hasMaterialRows(api, { id: 'solid', type: 'solid', d: {} })).toBe(false);
    expect(hasMaterialRows({ services: { get: () => null } } as any, model())).toBe(false);
    expect(isMaterialPropKey('mi.0.opacity')).toBe(true);
    expect(isMaterialPropKey('m.roughness')).toBe(false);
  });

  it('maps each source layer to the models that use it, refreshing when links change', () => {
    const layer = model({ emissive: { layerId: 'title' }, color: 'asset-id' }, { 0: { color: { layerId: 'paint' } }, 1: { color: { layerId: 'paint' } } });
    const { api } = apiFor([layer, { id: 'paint', type: 'solid' }]);
    expect([...materialSourceMap(api)]).toEqual([['title', ['model']], ['paint', ['model']]]);
    layer.d.data.object.sourceMaterials = {};
    expect([...materialSourceMap(api)]).toEqual([['title', ['model']]]);
  });

  it('files a 3D base-material row under Material, not under a mask', () => {
    expect(propertySection({}, { key: 'm.roughness', group: 'Material' }, String).label).toBe('Material');
    expect(propertySection({}, { key: 'm.mask1.feather', group: 'Mask 1' }, String).label).toBe('Mask 1');
  });
});
