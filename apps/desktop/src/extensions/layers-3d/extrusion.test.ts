import { describe, expect, it, vi } from 'vitest';
import { materialEntries, materialUsage } from './materials';
import { isExtrudable, isExtruded, setExtruded, surfaceEntries, surfacesMatchFront, useFrontForAll, resetSurface, installSurfaceProperty, BEVEL_STYLES, SURFACE_PROPERTY_SPECS } from './extrusion';

const channel = (v: unknown, kf: any[] = []) => ({ v, kf, expr: null });
function harness(extrusion: any = { version: 1, enabled: true, p: {}, surfaces: {} }, threeD = true) {
  const title = { id: 'title', type: 'text', name: 'Title', from: 1, threeD, d: { text: 'Hi', ...(extrusion ? { extrusion } : {}) } };
  const layers: any[] = [title, { id: 'clip', type: 'video', name: 'Clip' }, { id: 'note', type: 'solid', name: 'Note' }];
  const api: any = {
    model: { layer: (id: string) => layers.find(layer => layer.id === id) ?? null },
    project: { get: () => ({ layers, assets: {} }) },
    scene3d: {
      listMaterials: () => [],
      setExtrusion: vi.fn(() => ({ ok: true })),
      setSurfaceMaterialMap: vi.fn(() => ({ ok: true })),
      setSurfaceMaterialProperties: vi.fn(() => ({ ok: true })),
    },
    edit: { begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(), apply: vi.fn(() => ({ ok: true })) },
    transport: { invalidate: vi.fn(), time: () => 0 },
    ui: { toast: vi.fn() },
  };
  return { api, title, layers };
}

describe('extruded text and shape surfaces', () => {
  it('lists Front, Bevel, Side and Back as materials 0–3 once extrusion is on', () => {
    const { api } = harness({ version: 1, enabled: true, p: {}, surfaces: { side: { maps: { color: { layerId: 'clip' } }, properties: {} } } });
    expect(materialEntries(api, 'title')).toEqual([
      { index: 0, name: 'Front', maps: {} }, { index: 1, name: 'Bevel', maps: {} },
      { index: 2, name: 'Side', maps: { color: { layerId: 'clip' } } }, { index: 3, name: 'Back', maps: {} },
    ]);
    expect(surfaceEntries(api, 'title')).toHaveLength(4);
  });

  it('has no surfaces while extrusion is off or the layer is 2D', () => {
    expect(surfaceEntries(harness({ version: 1, enabled: false, p: {}, surfaces: {} }).api, 'title')).toEqual([]);
    expect(materialEntries(harness(null).api, 'title')).toEqual([]);
    expect(surfaceEntries(harness({ version: 1, enabled: true, p: {}, surfaces: {} }, false).api, 'title')).toEqual([]);
    expect(isExtrudable({ type: 'shape' } as any)).toBe(true);
    expect(isExtrudable({ type: 'image' } as any)).toBe(false);
    expect(isExtruded({ type: 'text', threeD: true, d: { extrusion: { enabled: true } } } as any)).toBe(true);
  });

  it('first Extrude gives visible depth and names the 3D step; later toggles keep authored settings', () => {
    const { api, title } = harness(null, false);
    setExtruded(api, 'title', true);
    expect(api.scene3d.setExtrusion).toHaveBeenLastCalledWith('title', { enabled: true, depth: 40 }, { label: 'Make 3D and Extrude', origin: 'inspector' });
    title.threeD = true; (title.d as any).extrusion = { enabled: false, p: { depth: channel(90) } };
    setExtruded(api, 'title', true);
    expect(api.scene3d.setExtrusion).toHaveBeenLastCalledWith('title', { enabled: true }, { label: 'Extrude', origin: 'inspector' });
    setExtruded(api, 'title', false);
    expect(api.scene3d.setExtrusion).toHaveBeenLastCalledWith('title', { enabled: false }, { label: 'Turn off extrusion', origin: 'inspector' });
  });

  it('reports a refused Extrude instead of failing silently', () => {
    const { api } = harness();
    api.scene3d.setExtrusion.mockReturnValueOnce({ ok: false, message: 'Layer is locked' });
    expect(setExtruded(api, 'title', true)).toBe(false);
    expect(api.ui.toast).toHaveBeenCalledWith('Layer is locked', { error: true, key: 'extrude' });
  });

  it('Use Front for All copies maps and keyframed options, clearing what Front lacks, in one step', () => {
    const front = { maps: { color: { layerId: 'clip' } }, properties: { roughness: channel(.2, [{ t: 0, v: .2 }, { t: 1, v: .8 }]) } };
    const { api } = harness({ version: 1, enabled: true, p: {}, surfaces: { front, side: { maps: { emissive: 'img' }, properties: { color: channel('#FF0000') } } } });
    expect(surfacesMatchFront(api, 'title')).toBe(false);
    expect(useFrontForAll(api, 'title')).toBe(true);
    expect(api.edit.begin).toHaveBeenCalledWith('Use Front for All', { origin: 'inspector' });
    expect(api.edit.commit).toHaveBeenCalledWith('Use Front for All');
    const maps = api.scene3d.setSurfaceMaterialMap.mock.calls.map((call: any[]) => call.slice(1, 4));
    expect(maps).toEqual(expect.arrayContaining([['bevel', 'color', { layerId: 'clip' }], ['side', 'color', { layerId: 'clip' }], ['side', 'emissive', null], ['back', 'color', { layerId: 'clip' }]]));
    const side = api.scene3d.setSurfaceMaterialProperties.mock.calls.find((call: any[]) => call[1] === 'side')[2];
    expect(side).toEqual({ color: null, roughness: { v: .2, kf: [{ t: 0, v: .2 }, { t: 1, v: .8 }], expr: null } });
  });

  it('cancels the whole copy when one surface is refused', () => {
    const { api } = harness({ version: 1, enabled: true, p: {}, surfaces: { front: { maps: { color: 'img' }, properties: {} } } });
    api.scene3d.setSurfaceMaterialMap.mockReturnValueOnce({ ok: true }).mockReturnValueOnce({ ok: false, message: 'nope' });
    expect(useFrontForAll(api, 'title')).toBe(false);
    expect(api.edit.cancel).toHaveBeenCalled();
    expect(api.edit.commit).not.toHaveBeenCalled();
  });

  it('Reset clears one surface’s sources and overrides together', () => {
    const { api } = harness({ version: 1, enabled: true, p: {}, surfaces: { back: { maps: { color: 'img' }, properties: { opacity: channel(40) } } } });
    expect(resetSurface(api, 'title', 'back')).toBe(true);
    expect(api.scene3d.setSurfaceMaterialMap).toHaveBeenCalledWith('title', 'back', 'color', null, { label: 'Reset Back' });
    expect(api.scene3d.setSurfaceMaterialProperties).toHaveBeenCalledWith('title', 'back', { opacity: null }, { label: 'Reset Back' });
    expect(resetSurface(api, 'title', 'front')).toBe(false);
  });

  it('installs a surface option at its current value, or keyed in layer time', () => {
    const { api } = harness();
    installSurfaceProperty(api, 'title', 'side', 'metallic', .3, 2.5);
    expect(api.scene3d.setSurfaceMaterialProperties).toHaveBeenCalledWith('title', 'side', { metallic: { v: .3, kf: [{ t: 1.5, v: .3 }], expr: null } }, { label: 'Add keyframe for Side Metallic', origin: 'inspector' });
  });

  it('a layer painted on a surface reports where it is used', () => {
    const { api } = harness({ version: 1, enabled: true, p: {}, surfaces: { front: { maps: { color: { layerId: 'clip' }, emissive: { layerId: 'clip' } }, properties: {} } } });
    expect(materialUsage(api, 'clip')).toEqual([{ layerId: 'title', name: 'Title', uses: ['Front · Base Color', 'Front · Emissive'] }]);
  });

  it('every bevel profile and surface option has a plain-language tip', () => {
    expect(BEVEL_STYLES.map(style => style.id)).toEqual(['none', 'angular', 'concave', 'convex']);
    for (const style of BEVEL_STYLES) expect(style.tip).toMatch(new RegExp(`^${style.label} — [A-Z]`));
    expect(SURFACE_PROPERTY_SPECS.map(spec => spec.key)).toEqual(['color', 'opacity', 'roughness', 'metallic', 'emissionColor', 'emissionIntensity', 'ambientResponse']);
  });
});
