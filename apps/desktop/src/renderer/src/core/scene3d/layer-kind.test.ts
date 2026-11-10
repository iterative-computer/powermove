import { describe, expect, it } from 'vitest';
import { layer3DKind, layerCanLink } from './layers';

const object = (data: any) => ({ type: 'extension', d: { definition: 'powermove.3d.object', data: { object: data } } });
const light = (type: string) => ({ type: 'extension', d: { definition: 'powermove.3d.light', data: { light: { type } } } });

describe('layer3DKind', () => {
  it('names each environment layer for what it is, not "3D Model"', () => {
    expect(layer3DKind(object({ world: {} }))).toEqual({ label: 'World', icon: 'globe', surrounds: true });
    expect(layer3DKind(object({ terrain: {} }))).toEqual({ label: 'Terrain', icon: 'mountains' });
    expect(layer3DKind(object({ scatter: {} }))).toEqual({ label: 'Scatter', icon: 'tree' });
    expect(layer3DKind(object({ fog: {} }))).toEqual({ label: 'Fog Volume', icon: 'fog' });
    expect(layer3DKind(object({ source: { primitive: 'box' } }))).toEqual({ label: '3D Model', icon: 'cube' });
  });
  it('names lights by type and cameras with an icon that exists', () => {
    expect(layer3DKind(light('ambient'))?.label).toBe('Ambient Light');
    expect(layer3DKind(light('environment'))?.label).toBe('Environment Light');
    expect(layer3DKind(light('area'))?.label).toBe('Softbox Light');
    expect(layer3DKind({ type: 'extension', d: { definition: 'powermove.3d.camera', data: {} } })).toEqual({ label: 'Camera', icon: 'cam' });
  });
  it('marks the layers that surround the scene: no place, so no Transform or Link Layers', () => {
    expect(layer3DKind(light('ambient'))?.surrounds).toBe(true);
    expect(layer3DKind(light('environment'))?.surrounds).toBe(true);
    expect(layer3DKind(light('spot'))?.surrounds).toBeUndefined();
    expect(layer3DKind(object({ fog: {} }))?.surrounds).toBeUndefined();
    const PM = { TYPE_META: { extension: {}, audio: { transform: false } } };
    expect(layerCanLink(PM, light('environment'))).toBe(false);
    expect(layerCanLink(PM, object({ world: {} }))).toBe(false);
    expect(layerCanLink(PM, light('point'))).toBe(true);
    expect(layerCanLink(PM, { type: 'audio' })).toBe(false);
  });
  it('leaves other layers to their own type', () => {
    expect(layer3DKind({ type: 'text', d: {} })).toBeNull();
  });
});
