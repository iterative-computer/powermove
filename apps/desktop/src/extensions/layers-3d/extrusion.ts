import type { Layer, PowermoveAPI } from 'powermove';
import { MATERIAL_PROPERTY_SPECS, materialEntries, type MaterialEntry, type MaterialMapValue, type MaterialSlot, type PropertySpec } from './materials';

/*
 * Extruded text and shapes, as in After Effects' Advanced 3D: Geometry
 * Options give a text or shape layer real depth and bevels, and Material
 * Options paint its Front, Bevel, Side and Back separately. Everything goes
 * through the public scene3d API, so undo, autokey and agents stay shared.
 */

export type Surface = 'front' | 'bevel' | 'side' | 'back';
export const SURFACES: readonly Surface[] = ['front', 'bevel', 'side', 'back'];
export const SURFACE_NAMES: Record<Surface, string> = { front: 'Front', bevel: 'Bevel', side: 'Side', back: 'Back' };
export const SURFACE_TIPS: Record<Surface, string> = {
  front: 'Front — The face you see head-on, with the layer’s own outline.',
  bevel: 'Bevel — The angled or rounded edge between the front and the sides. Only shows with a bevel style.',
  side: 'Side — The walls the extrusion depth adds.',
  back: 'Back — The face at the far end, seen when the layer turns around.'
};

export type BevelStyle = 'none' | 'angular' | 'concave' | 'convex';
export interface BevelSpec { id: BevelStyle; label: string; tip: string; path: string }
/** Small cross-section icons: a block whose top-right corner takes the profile. */
export const BEVEL_STYLES: readonly BevelSpec[] = [
  { id: 'none', label: 'None', tip: 'None — A crisp, square edge.', path: 'M3 13V3h10v10z' },
  { id: 'angular', label: 'Angular', tip: 'Angular — The edge is cut off flat, like a chamfer.', path: 'M3 13V3h5l5 5v5z' },
  { id: 'concave', label: 'Concave', tip: 'Concave — The edge curves inward, like a scooped molding.', path: 'M3 13V3h5a5 5 0 0 0 5 5v5z' },
  { id: 'convex', label: 'Convex', tip: 'Convex — The edge is rounded outward, like a pebble.', path: 'M3 13V3h5a5 5 0 0 1 5 5v5z' }
];

export type Quality = 'draft' | 'standard' | 'high';
export const QUALITY_OPTIONS: ReadonlyArray<{ v: Quality; label: string }> = [
  { v: 'draft', label: 'Draft' }, { v: 'standard', label: 'Standard' }, { v: 'high', label: 'High' }
];

export interface GeometrySpec { key: 'depth' | 'bevelDepth' | 'holeBevelDepth'; label: string; kind: 'num'; step: number; min: number; max?: number; unit: string; tip: string }
export const GEOMETRY_TIPS = {
  extrude: 'Extrude — Gives the layer real depth, with bevels and its own material for each side. Lights, shadows and the camera see it as solid.',
  extrude2D: 'Extrude — Extrusion works on 3D layers, so turning this on makes the layer 3D too, in the same step. It gains real depth, bevels and a material for each side.',
  bevelStyle: 'Bevel Style — The shape of the edge where the front meets the sides.',
  quality: 'Geometry Quality — How finely curves are divided. Draft is fastest while you work; High is smoothest for close-ups.'
};
export const GEOMETRY_FIELDS: readonly GeometrySpec[] = [
  { key: 'depth', label: 'Depth', kind: 'num', step: 1, min: 0, unit: 'px', tip: 'Extrusion Depth — How far the layer reaches back from its front face, in pixels.' },
  { key: 'bevelDepth', label: 'Bevel Depth', kind: 'num', step: 0.5, min: 0, unit: 'px', tip: 'Bevel Depth — How wide the beveled edge is. It never grows past half the extrusion depth.' },
  { key: 'holeBevelDepth', label: 'Hole Bevel', kind: 'num', step: 1, min: 0, max: 100, unit: '%', tip: 'Hole Bevel Depth — The bevel inside holes, like the middle of an O or a B, as a share of the outer bevel.' }
];

export type SurfaceKey = 'color' | PropertySpec['key'];
export interface SurfacePropertySpec extends Omit<PropertySpec, 'key'> { key: SurfaceKey }
/** A surface adds its own Color (the layer’s fill until changed) before the six Material Options. */
export const SURFACE_PROPERTY_SPECS: readonly SurfacePropertySpec[] = [
  { key: 'color', label: 'Color', kind: 'color', tip: 'The surface’s own color, used when Base Color has no source. Starts as the layer’s fill.' },
  ...MATERIAL_PROPERTY_SPECS
];

export const isExtrudable = (layer: Layer | null | undefined): boolean => layer?.type === 'text' || layer?.type === 'shape';
const extrusionOf = (layer: Layer | null | undefined): any => (layer?.d as any)?.extrusion;
/** Extrusion is on: the surfaces and Geometry Options are live. */
export const isExtruded = (layer: Layer | null | undefined): boolean =>
  isExtrudable(layer) && (layer as any).threeD === true && extrusionOf(layer)?.enabled === true;

export const geometryPath = (key: string): string => `geo.${key}`;
export const surfacePath = (surface: Surface, key: SurfaceKey): string => `ms.${surface}.${key}`;
export const surfaceAt = (index: number | null): Surface => SURFACES[index ?? 0] ?? 'front';

/** The four surfaces, shaped like a model's materials (see materialEntries). */
export function surfaceEntries(api: PowermoveAPI, layerId: string): MaterialEntry[] {
  return isExtruded(api.model.layer(layerId)) ? materialEntries(api, layerId) : [];
}

/** The live channel once installed; null while the property still inherits. */
export function surfaceChannel(layer: Layer | null | undefined, surface: Surface, key: SurfaceKey): any {
  return extrusionOf(layer)?.surfaces?.[surface]?.properties?.[key] ?? null;
}

export function geometryChannel(layer: Layer | null | undefined, key: string): any {
  return extrusionOf(layer)?.p?.[key] ?? null;
}

const specLabel = (key: SurfaceKey) => SURFACE_PROPERTY_SPECS.find(spec => spec.key === key)?.label ?? key;

/** Install a surface property with its current value (and optionally a key at comp time). One undo step. */
export function installSurfaceProperty(api: PowermoveAPI, layerId: string, surface: Surface, key: SurfaceKey, value: unknown, keyAt?: number) {
  const layer = api.model.layer(layerId);
  const patch = keyAt == null ? { [key]: value } : { [key]: { v: value, kf: [{ t: Math.max(0, keyAt - (layer?.from ?? 0)), v: value }], expr: null } };
  return api.scene3d.setSurfaceMaterialProperties(layerId, surface, patch as any, { label: keyAt == null ? `Set ${SURFACE_NAMES[surface]} ${specLabel(key)}` : `Add keyframe for ${SURFACE_NAMES[surface]} ${specLabel(key)}`, origin: 'inspector' });
}

export function surfaceOverrideCount(api: PowermoveAPI, layerId: string, surface: Surface): number {
  const layer = api.model.layer(layerId), material = extrusionOf(layer)?.surfaces?.[surface];
  return Object.keys(material?.maps || {}).length + Object.keys(material?.properties || {}).length;
}

/** Turn extrusion on or off. Turning it on also makes a 2D layer 3D, in the same undo step. */
export function setExtruded(api: PowermoveAPI, layerId: string, on: boolean): boolean {
  const layer = api.model.layer(layerId);
  const first = on && !extrusionOf(layer);
  // A first extrusion starts with visible depth, so the change reads at once.
  const patch = first ? { enabled: true, depth: 40 } : { enabled: on };
  const label = on ? ((layer as any)?.threeD ? 'Extrude' : 'Make 3D and Extrude') : 'Turn off extrusion';
  const result = api.scene3d.setExtrusion(layerId, patch, { label, origin: 'inspector' });
  if (!result.ok) { api.ui.toast(result.message, { error: true, key: 'extrude' }); return false; }
  api.transport.invalidate();
  return true;
}

/** A discrete geometry choice (Bevel Style, Quality), keyed when animated or autokey is on. */
export function setGeometryChoice(api: PowermoveAPI, layerId: string, key: 'bevelStyle' | 'quality', value: string, label: string): boolean {
  const result = api.edit.apply({ type: 'set_property', target: layerId, path: geometryPath(key), value, time: api.transport.time(), mode: 'auto', preserveHandEdits: false }, { label, origin: 'inspector' });
  if (!result.ok) { api.ui.toast(result.message, { error: true, key: 'extrude' }); return false; }
  api.transport.invalidate();
  return true;
}

/** Assign (or clear) one surface slot. */
export function assignSurfaceMap(api: PowermoveAPI, layerId: string, surface: Surface, slot: MaterialSlot, map: MaterialMapValue | null): boolean {
  const result = api.scene3d.setSurfaceMaterialMap(layerId, surface, slot, map as any, { label: `Set ${SURFACE_NAMES[surface]} source`, origin: 'inspector' });
  if (!result.ok) { api.ui.toast(result.message, { error: true, key: 'material-assign' }); return false; }
  api.transport.invalidate();
  return true;
}

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));

/** Put one surface back to the layer's own look: no sources, no overrides. */
export function resetSurface(api: PowermoveAPI, layerId: string, surface: Surface): boolean {
  const material = extrusionOf(api.model.layer(layerId))?.surfaces?.[surface];
  const slots = Object.keys(material?.maps || {}) as MaterialSlot[], keys = Object.keys(material?.properties || {});
  if (!slots.length && !keys.length) return false;
  return grouped(api, `Reset ${SURFACE_NAMES[surface]}`, label => {
    for (const slot of slots) check(api.scene3d.setSurfaceMaterialMap(layerId, surface, slot, null, { label }));
    if (keys.length) check(api.scene3d.setSurfaceMaterialProperties(layerId, surface, Object.fromEntries(keys.map(key => [key, null])) as any, { label }));
  });
}

/** Whether Bevel, Side and Back already look exactly like Front. */
export function surfacesMatchFront(api: PowermoveAPI, layerId: string): boolean {
  const surfaces = extrusionOf(api.model.layer(layerId))?.surfaces || {};
  const of = (surface: Surface) => JSON.stringify({ maps: surfaces[surface]?.maps || {}, properties: surfaces[surface]?.properties || {} });
  const front = of('front');
  return SURFACES.every(surface => of(surface) === front);
}

/** Copy Front's sources and options, keyframes included, onto Bevel, Side and Back. One undo step. */
export function useFrontForAll(api: PowermoveAPI, layerId: string): boolean {
  const layer = api.model.layer(layerId), surfaces = extrusionOf(layer)?.surfaces || {};
  const front = surfaces.front || { maps: {}, properties: {} };
  return grouped(api, 'Use Front for All', label => {
    for (const surface of SURFACES.slice(1)) {
      const own = surfaces[surface] || { maps: {}, properties: {} };
      for (const slot of new Set([...Object.keys(own.maps || {}), ...Object.keys(front.maps || {})]) as Set<MaterialSlot>)
        check(api.scene3d.setSurfaceMaterialMap(layerId, surface, slot, front.maps?.[slot] == null ? null : clone(front.maps[slot]), { label }));
      const patch: Record<string, unknown> = {};
      for (const key of Object.keys(own.properties || {})) if (!front.properties?.[key]) patch[key] = null;
      for (const [key, channel] of Object.entries<any>(front.properties || {})) patch[key] = clone({ v: channel.v, kf: channel.kf || [], expr: channel.expr ?? null });
      if (Object.keys(patch).length) check(api.scene3d.setSurfaceMaterialProperties(layerId, surface, patch as any, { label }));
    }
  });
}

function check(result: { ok: boolean; message?: string }): void { if (!result.ok) throw new Error(result.message); }
function grouped(api: PowermoveAPI, label: string, run: (label: string) => void): boolean {
  api.edit.begin(label, { origin: 'inspector' });
  try {
    run(label);
    api.edit.commit(label);
    api.transport.invalidate();
    return true;
  } catch (error) {
    api.edit.cancel();
    api.ui.toast(error instanceof Error ? error.message : String(error), { error: true });
    return false;
  }
}
