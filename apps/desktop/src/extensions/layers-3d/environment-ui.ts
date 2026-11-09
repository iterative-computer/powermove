import type { PowermoveAPI } from 'powermove';
import type { FieldSpec } from './scene-model';

/* Fields, presets and creation flows for World, Fog Volume, Terrain, Scatter
   and AE's Material Options. Every edit goes through api.scene3d.edit, the
   same guarded operations agents use. */

export type SkyPreset = 'clear-noon' | 'golden-hour' | 'overcast' | 'blue-hour' | 'night';

/** Preset picker entries. `sky` is the zenith → horizon gradient and `sun`
    the disc color; both are content (a picture of the sky), not chrome. */
export const SKY_PRESETS: ReadonlyArray<{ id: SkyPreset; label: string; sky: [string, string, string]; sun: string | null; elevation: number }> = [
  { id: 'clear-noon', label: 'Clear noon', sky: ['#3E7FD0', '#8DBCEB', '#DCEBF7'], sun: '#FFFBEA', elevation: 45 },
  { id: 'golden-hour', label: 'Golden hour', sky: ['#5C7DB3', '#E9B07A', '#FFD49A'], sun: '#FFE2A8', elevation: 6 },
  { id: 'overcast', label: 'Overcast', sky: ['#9AA3AD', '#C3C9CF', '#DDE0E3'], sun: null, elevation: 45 },
  { id: 'blue-hour', label: 'Blue hour', sky: ['#14244A', '#3D5C99', '#B5A6C9'], sun: null, elevation: -5 },
  { id: 'night', label: 'Night', sky: ['#04060D', '#0B1426', '#1C2740'], sun: null, elevation: -20 }
];

/** The sun/haze values `set_world` applies for each preset (an Environment light applies them itself). */
export const SKY_PRESET_VALUES: Record<SkyPreset, Record<string, number>> = {
  'clear-noon': { elevation: 45, haze: 2, overcast: 0, sunIntensity: 1 }, 'golden-hour': { elevation: 6, haze: 3, overcast: 0, sunIntensity: 1 },
  'overcast': { elevation: 45, haze: 4, overcast: 1, sunIntensity: 0.1 }, 'blue-hour': { elevation: -5, haze: 2, overcast: 0, sunIntensity: 0 },
  'night': { elevation: -20, haze: 2, overcast: 0, sunIntensity: 0 }
};

export const WORLD_MODES = [
  { v: 'sky', label: 'Sky' },
  { v: 'hdri', label: 'Image' },
  { v: 'none', label: 'None' }
] as const;

export const WORLD_LIGHT_FIELDS: FieldSpec[] = [
  { key: 'intensity', label: 'Intensity', kind: 'num', step: 0.05, min: 0, max: 10 },
  { key: 'rotation', label: 'Rotation', kind: 'num', step: 1, unit: '°' },
  { key: 'visible', label: 'Visible to camera', kind: 'toggle' }
];
export const WORLD_SUN_FIELDS: FieldSpec[] = [
  { key: 'elevation', label: 'Sun height', kind: 'num', step: 0.5, min: -90, max: 90, unit: '°' },
  { key: 'azimuth', label: 'Sun direction', kind: 'num', step: 1, unit: '°' },
  { key: 'sunIntensity', label: 'Sun strength', kind: 'num', step: 0.05, min: 0, max: 10 },
  { key: 'haze', label: 'Haze', kind: 'num', step: 0.05, min: 1, max: 10 },
  { key: 'overcast', label: 'Clouds', kind: 'num', step: 0.01, min: 0, max: 1 },
  { key: 'groundColor', label: 'Ground color', kind: 'color' }
];
export const WORLD_FOG_FIELDS: FieldSpec[] = [
  { key: 'fogDensity', label: 'Fog', kind: 'num', step: 0.001, min: 0, max: 0.2 },
  { key: 'fogColor', label: 'Fog color', kind: 'color' },
  { key: 'fogHeightFalloff', label: 'Fog falloff', kind: 'num', step: 0.01, min: 0, max: 2 },
  { key: 'fogBaseHeight', label: 'Fog base', kind: 'num', step: 0.05 }
];
export const FOG_FIELDS: FieldSpec[] = [
  { key: 'shape', label: 'Shape', kind: 'select', options: [{ v: 'box', label: 'Box' }, { v: 'sphere', label: 'Sphere' }] },
  { key: 'width', label: 'Width', kind: 'num', step: 0.05, min: 0.0001 },
  { key: 'height', label: 'Height', kind: 'num', step: 0.05, min: 0.0001 },
  { key: 'depth', label: 'Depth', kind: 'num', step: 0.05, min: 0.0001 },
  { key: 'density', label: 'Density', kind: 'num', step: 0.001, min: 0, max: 1 },
  { key: 'color', label: 'Color', kind: 'color' },
  { key: 'anisotropy', label: 'Forward glow', kind: 'num', step: 0.01, min: -0.95, max: 0.95 }
];
export const TERRAIN_FIELDS: FieldSpec[] = [
  { key: 'width', label: 'Width', kind: 'num', step: 0.5, min: 0.001 },
  { key: 'depth', label: 'Depth', kind: 'num', step: 0.5, min: 0.001 },
  { key: 'amplitude', label: 'Height', kind: 'num', step: 0.05, min: 0, max: 20 },
  { key: 'frequency', label: 'Feature size', kind: 'num', step: 0.005, min: 0.01, max: 1 },
  { key: 'octaves', label: 'Detail', kind: 'num', step: 1, min: 1, max: 12 },
  { key: 'ridged', label: 'Ridges', kind: 'toggle' },
  { key: 'seed', label: 'Variation', kind: 'num', step: 1, min: 0 },
  { key: 'resolution', label: 'Resolution', kind: 'num', step: 1, min: 2, max: 256 }
];
export const REGION_FIELDS: FieldSpec[] = [
  { key: 'color', label: 'Color', kind: 'color' },
  { key: 'minSlope', label: 'Steeper than', kind: 'num', step: 1, min: 0, max: 90, unit: '°' },
  { key: 'maxSlope', label: 'Flatter than', kind: 'num', step: 1, min: 0, max: 90, unit: '°' },
  { key: 'minHeight', label: 'Above', kind: 'num', step: 0.05 },
  { key: 'maxHeight', label: 'Below', kind: 'num', step: 0.05 },
  { key: 'roughness', label: 'Roughness', kind: 'num', step: 0.01, min: 0.02, max: 1 }
];
export const SCATTER_FIELDS: FieldSpec[] = [
  { key: 'count', label: 'Count', kind: 'num', step: 10, min: 0, max: 100000 },
  { key: 'density', label: 'Density', kind: 'num', step: 0.01, min: 0, max: 1 },
  { key: 'seed', label: 'Variation', kind: 'num', step: 1, min: 0 },
  { key: 'scaleMin', label: 'Smallest', kind: 'num', step: 0.05, min: 0.0001 },
  { key: 'scaleMax', label: 'Largest', kind: 'num', step: 0.05, min: 0.0001 },
  { key: 'rotationMin', label: 'Turn from', kind: 'num', step: 1, unit: '°' },
  { key: 'rotationMax', label: 'Turn to', kind: 'num', step: 1, unit: '°' },
  { key: 'maxSlope', label: 'Flatter than', kind: 'num', step: 1, min: 0, max: 90, unit: '°' },
  { key: 'minSlope', label: 'Steeper than', kind: 'num', step: 1, min: 0, max: 90, unit: '°' },
  { key: 'minHeight', label: 'Above', kind: 'num', step: 0.05 },
  { key: 'maxHeight', label: 'Below', kind: 'num', step: 0.05 }
];
/** After Effects' Material Options, in its order and wording. */
export const MATERIAL_OPTION_FIELDS: FieldSpec[] = [
  { key: 'castsShadows', label: 'Casts Shadows', kind: 'select', options: [{ v: 'off', label: 'Off' }, { v: 'on', label: 'On' }, { v: 'only', label: 'Only' }] },
  { key: 'acceptsShadows', label: 'Accepts Shadows', kind: 'toggle' },
  { key: 'acceptsLights', label: 'Accepts Lights', kind: 'toggle' },
  { key: 'appearsInReflections', label: 'Appears in Reflections', kind: 'toggle' }
];
export const IRIS_FIELDS: FieldSpec[] = [
  { key: 'irisShape', label: 'Iris shape', kind: 'select', options: [{ v: 0, label: 'Round' }, ...[3, 4, 5, 6, 7, 8, 9, 10].map(n => ({ v: n, label: `${n} blades` }))] },
  { key: 'irisRotation', label: 'Iris rotation', kind: 'num', step: 1, unit: '°' },
  { key: 'irisRoundness', label: 'Iris roundness', kind: 'num', step: 0.01, min: 0, max: 1 },
  { key: 'irisAspect', label: 'Iris aspect ratio', kind: 'num', step: 0.01, min: 0.1, max: 10 }
];

/** The composition's World layer, if any. */
export const worldLayer = (api: PowermoveAPI): any => api.model.curComp().layers.find((l: any) => l.d?.data?.object?.world) ?? null;

export type EnvironmentKind = 'world' | 'fog' | 'terrain' | 'scatter';
export const environmentKind = (object: any): EnvironmentKind | null =>
  object?.world ? 'world' : object?.fog ? 'fog' : object?.terrain ? 'terrain' : object?.scatter ? 'scatter' : null;

/** Sun position on the top-down dome: zenith at the centre, horizon on the
    ring, below-horizon outside it. Azimuth 0 points at +Z (toward the
    default camera), drawn at the bottom. */
export function domePoint(azimuth: number, elevation: number, radius: number): { x: number; y: number } {
  const r = radius * Math.min(1.2, (90 - elevation) / 90), a = (azimuth * Math.PI) / 180;
  return { x: Math.sin(a) * r, y: Math.cos(a) * r };
}
export function domeAngles(x: number, y: number, radius: number): { azimuth: number; elevation: number } {
  const r = Math.min(1.2, Math.hypot(x, y) / radius);
  const azimuth = Math.round((Math.atan2(x, y) * 180) / Math.PI);
  return { azimuth, elevation: Math.round(Math.max(-18, 90 - r * 90) * 2) / 2 };
}

function run(api: PowermoveAPI, args: any, label: string): string | null {
  const result = api.scene3d.edit(args, { label, origin: 'command' });
  if (!result.ok) { api.ui.toast(result.message, { error: true }); return null; }
  const id = (result.data.result as any)?.id ?? null;
  if (id) api.selection.select([id]);
  api.transport.invalidate();
  return id;
}

/** A World with a good-looking default: golden-hour sky, light haze. */
export function addWorld(api: PowermoveAPI, preset: SkyPreset = 'golden-hour'): string | null {
  const existing = worldLayer(api);
  if (existing) { api.selection.select([existing.id]); return existing.id; }
  return run(api, { operation: 'set_world', world: { mode: 'sky', preset, p: { azimuth: -60, intensity: 1 } } }, 'Add World');
}

export function addFog(api: PowermoveAPI): string | null {
  return run(api, { operation: 'add_fog', fog: { p: { shape: 'box', width: 4, height: 2, depth: 4, density: 0.03, color: '#E6E1DA', anisotropy: 0.4 } } }, 'Add Fog Volume');
}

/** Rolling hills: grass on gentle ground, rock on steep slopes, light grass on
    top. In a composition without a sky or lights it also adds a golden-hour
    World, and without a camera one that frames the land, in one Undo, so the
    first frame already reads as a landscape. */
export function addTerrain(api: PowermoveAPI): string | null {
  const layers = api.model.curComp().layers as any[];
  const lit = layers.some(l => l.d?.data?.object?.world || l.d?.data?.light);
  const framed = layers.some(l => l.d?.data?.camera);
  const label = 'Add Terrain';
  api.edit.begin(label, { origin: 'command' });
  try {
    const result = api.scene3d.edit({
      operation: 'add_terrain',
      terrain: {
        p: { width: 30, depth: 30, resolution: 96, amplitude: 3, frequency: 0.12, octaves: 5, seed: 41, ridged: false },
        regions: [
          { p: { minSlope: 32, color: '#8C7D64', roughness: 0.9 } },
          { p: { minHeight: 1.4, color: '#A3A078' } },
          { p: { maxHeight: 1.4, color: '#687348' } }
        ]
      }
    } as any, { origin: 'command' });
    if (!result.ok) throw new Error(result.message);
    if (!lit) {
      const world = api.scene3d.edit({ operation: 'set_world', world: { mode: 'sky', preset: 'golden-hour', p: { elevation: 14, azimuth: -60, haze: 2.7, fogDensity: 0.003, fogColor: '#E7D2B8', fogHeightFalloff: 0.14, fogDistance: 150 } } } as any, { origin: 'command' });
      if (!world.ok) throw new Error(world.message);
    }
    if (!framed) {
      const camera = api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 18, y: 12, z: 23, targetX: 0, targetY: 0, targetZ: 0, fov: 45 } } } as any, { origin: 'command' });
      if (!camera.ok) throw new Error(camera.message);
    }
    const committed = api.edit.commit(label);
    if (!committed.ok) throw new Error(committed.message);
    const id = (result.data.result as any)?.id ?? null;
    if (id) api.selection.select([id]);
    api.transport.invalidate();
    return id;
  } catch (error) {
    api.edit.cancel();
    api.ui.toast(error instanceof Error ? error.message : String(error), { error: true });
    return null;
  }
}

/** Candidates for Scatter: what to copy (models) and what to cover (any model or terrain). */
export function scatterSources(api: PowermoveAPI): { models: { id: string; name: string }[]; surfaces: { id: string; name: string; terrain: boolean }[] } {
  const layers = api.model.curComp().layers as any[];
  const usable = layers.filter(l => { const o = l.d?.data?.object; return o && !o.world && !o.fog && !o.scatter; });
  return {
    models: usable.filter(l => !l.d.data.object.terrain).map(l => ({ id: l.id, name: l.name })),
    surfaces: usable.map(l => ({ id: l.id, name: l.name, terrain: !!l.d.data.object.terrain }))
  };
}

export function addScatter(api: PowermoveAPI, model: string, surface: string): string | null {
  const terrain = (api.model.layer(surface) as any)?.d?.data?.object?.terrain;
  const area = terrain ? terrain.p.width.v * terrain.p.depth.v : 4;
  const count = Math.max(20, Math.min(600, Math.round(area * 0.5)));
  // The model you copy becomes the pattern: hide it in the same Undo, so the
  // first frame shows the copies, not one big original in the middle. The
  // copies still render from a hidden source; its eye shows it again.
  const label = 'Add Scatter';
  api.edit.begin(label, { origin: 'command' });
  try {
    const result = api.scene3d.edit({
      operation: 'add_scatter',
      scatter: { model, surface, p: { count, density: 1, seed: 7, scaleMin: 0.7, scaleMax: 1.3, rotationMin: 0, rotationMax: 360, minSlope: 0, maxSlope: 35 } }
    } as any, { origin: 'command' });
    if (!result.ok) throw new Error(result.message);
    const source = api.model.layer(model) as any;
    if (source && !source.lock && source.on !== false) {
      const hidden = api.edit.dispatch({ type: 'set_layer', target: model, patch: { visible: false } } as any);
      if (!hidden.ok) throw new Error(hidden.message);
    }
    const committed = api.edit.commit(label);
    if (!committed.ok) throw new Error(committed.message);
    const id = (result.data.result as any)?.id ?? null;
    if (id) api.selection.select([id]);
    api.transport.invalidate();
    return id;
  } catch (error) {
    api.edit.cancel();
    api.ui.toast(error instanceof Error ? error.message : String(error), { error: true });
    return null;
  }
}

export function addAmbientLight(api: PowermoveAPI): string | null {
  return run(api, { operation: 'add_light', light: { type: 'ambient', name: 'Ambient Light', p: { intensity: 0.2, color: '#DCE5FF' } } }, 'Add Ambient Light');
}
export function addEnvironmentLight(api: PowermoveAPI): string | null {
  return run(api, { operation: 'add_light', light: { type: 'environment', name: 'Environment Light', world: { mode: 'sky', preset: 'clear-noon' }, p: { intensity: 1 } } }, 'Add Environment Light');
}

/** Choose an HDR, EXR or ordinary image and return its asset id. */
export async function pickEnvironmentImage(api: PowermoveAPI): Promise<string | null> {
  try {
    const [file] = await api.assets.pick({ accept: '.hdr,.exr,.png,.jpg,.jpeg,.webp', multiple: false });
    if (!file) return null;
    return (await api.assets.import(file)).id;
  } catch (error) {
    api.ui.toast(error instanceof Error ? error.message : String(error), { error: true });
    return null;
  }
}

/** Compositions other than the open one, and same-composition picture layers, for World/terrain sources. */
export function environmentSourceChoices(api: PowermoveAPI): { v: string; label: string }[] {
  const project: any = api.project.get(), current = api.model.curComp() as any;
  const comps = Object.entries<any>(project.comps || {}).filter(([id]) => id !== project.compId && id !== current?.id).map(([id, comp]) => ({ v: `comp:${id}`, label: `${comp.name || 'Composition'} (composition)` }));
  const layers = (current.layers as any[]).filter(l => ['image', 'video', 'precomp', 'solid'].includes(l.type)).map(l => ({ v: `layer:${l.id}`, label: `${l.name} (layer)` }));
  return [...comps, ...layers];
}
export const sourceKey = (source: any): string =>
  !source ? '' : typeof source === 'string' ? `asset:${source}` : source.compositionId ? `comp:${source.compositionId}` : source.layerId ? `layer:${source.layerId}` : '';
export const sourceFromKey = (key: string): any => {
  const [kind, id] = [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)];
  return kind === 'asset' ? id : kind === 'comp' ? { compositionId: id } : kind === 'layer' ? { layerId: id } : null;
};
