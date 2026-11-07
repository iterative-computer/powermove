import { describe, expect, it, vi } from 'vitest';
import { choiceLabel, extrusionMetadata, extrusionSectionRows, hasExtrusionRows, isExtrusionPropKey } from './extrusion-rows';
import { materialSourceMap } from './material-rows';
import { propertyMetadata } from './property-values';

const channel = (v: unknown, kf: any[] = []) => ({ v, kf, expr: null });
const titleLayer = (surfaces: any = {}) => ({
  id: 'title', type: 'text', name: 'Title', threeD: true,
  d: { extrusion: { enabled: true, p: { depth: channel(40, [{ t: 0, v: 10 }]), bevelStyle: channel('convex'), bevelDepth: channel(2), holeBevelDepth: channel(100), quality: channel('standard') }, surfaces } },
});
function apiFor(layer: any) {
  const service = { propertyValues: vi.fn(() => ({ color: '#DAD7FF', opacity: 100, roughness: .4 })) };
  const find = (L: any, path: string) => {
    const geo = /^geo\.(\w+)$/.exec(path)?.[1], ms = /^ms\.(\w+)\.(\w+)$/.exec(path);
    return geo ? L.d.extrusion.p[geo] ?? null : ms ? L.d.extrusion.surfaces?.[ms[1]!]?.properties?.[ms[2]!] ?? null : null;
  };
  return { service, api: { services: { get: (name: string) => name === 'materials' ? service : null }, anim: { findProp: find }, transport: { time: () => 0 } } as any };
}

describe('Timeline rows for extruded text and shapes', () => {
  it('lists Geometry Options, then the four surfaces under Materials', () => {
    const layer = titleLayer({ bevel: { maps: {}, properties: { metallic: channel(.5, [{ t: 0, v: .5 }]) } } });
    const rows = extrusionSectionRows(apiFor(layer).api, layer, () => false);
    expect(rows.map(row => `${row.kind}:${row.label}`)).toEqual([
      'heading:Geometry Options', 'prop:Extrusion Depth', 'prop:Bevel Style', 'prop:Bevel Depth', 'prop:Hole Bevel Depth', 'prop:Geometry Quality',
      'heading:Materials', 'material:Front', 'material:Bevel', 'prop:Metallic', 'material:Side', 'material:Back',
    ]);
    expect(rows.find(row => row.label === 'Extrusion Depth').key).toBe('geo.depth');
    expect(rows.find(row => row.label === 'Metallic').key).toBe('ms.bevel.metallic');
  });

  it('an opened surface shows Emissive, Color and the six options, stand-ins until installed', () => {
    const layer = titleLayer();
    const rows = extrusionSectionRows(apiFor(layer).api, layer, (_l, index) => index === 2);
    const side = rows.filter(row => String(row.key).startsWith('ms.side.'));
    expect(side.map(row => row.label)).toEqual(['Color', 'Opacity', 'Roughness', 'Metallic', 'Emission Color', 'Emission Intensity', 'Ambient Response']);
    expect(side.every(row => row.virtual?.index === 2)).toBe(true);
    expect(side[0].prop.v).toBe('#DAD7FF');
    expect(rows.find(row => row.key === 'material:2:emissive')?.label).toBe('Emissive');
  });

  it('shows nothing for flat or non-extruded layers', () => {
    const layer = titleLayer();
    expect(extrusionSectionRows(apiFor(layer).api, { ...layer, threeD: false }, () => true)).toEqual([]);
    expect(hasExtrusionRows({ type: 'image', threeD: true, d: { extrusion: { enabled: true } } })).toBe(false);
    expect(hasExtrusionRows(layer)).toBe(true);
  });

  it('formats word values and drags with sensible steps', () => {
    expect(choiceLabel('geo.bevelStyle', 'convex')).toBe('Convex');
    expect(choiceLabel('geo.quality', 'high')).toBe('High');
    expect(choiceLabel('c.text', 'hello')).toBeNull();
    expect(isExtrusionPropKey('geo.depth')).toBe(true);
    expect(isExtrusionPropKey('ms.front.opacity')).toBe(true);
    expect(isExtrusionPropKey('mi.0.opacity')).toBe(false);
    expect(extrusionMetadata('geo.depth')).toEqual({ step: 1, min: 0, max: 10000 });
    expect(extrusionMetadata('ms.side.roughness')).toEqual({ step: .01, min: 0, max: 1 });
    expect(propertyMetadata({ model: {}, effects: {} } as any, {}, 'geo.holeBevelDepth')).toEqual({ step: 1, min: 0, max: 100 });
  });

  it('a layer used on a surface gets the material badge', () => {
    const layer = titleLayer({ front: { maps: { color: { layerId: 'clip' } }, properties: {} } });
    const map = materialSourceMap({ project: { get: () => ({ layers: [layer, { id: 'clip', type: 'video' }] }) } } as any);
    expect(map.get('clip')).toEqual(['title']);
  });
});
