import type { PowermoveAPI } from 'powermove';
import { materialsService, type MaterialRow } from './material-rows';

/*
 * An extruded text or shape layer in the Timeline, as in After Effects'
 * Advanced 3D: a Geometry Options group (depth, bevel, quality) and a
 * Materials group listing Front, Bevel, Side and Back with their Base Color
 * pop-up and, once a surface is opened, its Emissive source plus Color and the
 * six Material Options. The pop-ups and the first-edit install come from the
 * 3D Layers extension's `materials` service, which knows surfaces too.
 */

const SURFACES = ['front', 'bevel', 'side', 'back'] as const;
const SURFACE_NAMES = ['Front', 'Bevel', 'Side', 'Back'];
const GEOMETRY: ReadonlyArray<[string, string]> = [
  ['depth', 'Extrusion Depth'], ['bevelStyle', 'Bevel Style'], ['bevelDepth', 'Bevel Depth'], ['holeBevelDepth', 'Hole Bevel Depth'], ['quality', 'Geometry Quality']
];
const SURFACE_OPTIONS: ReadonlyArray<[string, string]> = [
  ['color', 'Color'], ['opacity', 'Opacity'], ['roughness', 'Roughness'], ['metallic', 'Metallic'],
  ['emissionColor', 'Emission Color'], ['emissionIntensity', 'Emission Intensity'], ['ambientResponse', 'Ambient Response']
];

export const isExtrudedLayer = (layer: any): boolean =>
  (layer?.type === 'text' || layer?.type === 'shape') && layer.threeD === true && layer.d?.extrusion?.enabled === true;
export const isExtrusionPropKey = (key: unknown): boolean => /^(geo|ms)\./.test(String(key ?? ''));

/** Word values (Bevel Style, Quality) read capitalized, as in the Properties panel. */
export const CHOICE_PROPERTIES: Record<string, readonly string[]> = {
  'geo.bevelStyle': ['none', 'angular', 'concave', 'convex'],
  'geo.quality': ['draft', 'standard', 'high']
};
export function choiceLabel(key: unknown, value: unknown): string | null {
  if (!CHOICE_PROPERTIES[String(key)] || typeof value !== 'string') return null;
  return value ? value[0]!.toUpperCase() + value.slice(1) : value;
}

/** Drag sensitivity and limits for Geometry and surface Material Options. */
export function extrusionMetadata(path: string): { step?: number; min?: number; max?: number } | null {
  const geo = /^geo\.(\w+)$/.exec(path)?.[1], option = /^ms\.\w+\.(\w+)$/.exec(path)?.[1];
  if (geo) return ({ depth: { step: 1, min: 0, max: 10000 }, bevelDepth: { step: .1, min: 0, max: 1000 }, holeBevelDepth: { step: 1, min: 0, max: 100 } } as Record<string, any>)[geo] ?? null;
  if (option) return ({ opacity: { step: 1, min: 0, max: 100 }, ambientResponse: { step: 1, min: 0, max: 100 }, roughness: { step: .01, min: 0, max: 1 }, metallic: { step: .01, min: 0, max: 1 }, emissionIntensity: { step: .1, min: 0, max: 100 } } as Record<string, any>)[option] ?? null;
  return null;
}

export const hasExtrusionRows = (layer: any): boolean => isExtrudedLayer(layer);

/** Geometry Options (always all five, like After Effects), then the surfaces. A
 * closed surface still lists its animated options, as a layer lists its keyframed
 * properties; options that still follow the layer are stand-in rows (`virtual`). */
export function extrusionSectionRows(api: Pick<PowermoveAPI, 'services' | 'anim' | 'transport'>, layer: any, isOpen: (layer: any, index: number | null) => boolean): any[] {
  if (!isExtrudedLayer(layer)) return [];
  const rows: any[] = [{ kind: 'heading', L: layer, key: 'section:geometry', label: 'Geometry Options' }];
  for (const [key, label] of GEOMETRY) {
    const path = `geo.${key}`, prop = api.anim.findProp(layer, path) as any;
    if (prop) rows.push({ kind: 'prop', L: layer, key: path, prop, label, group: 'Geometry Options' });
  }
  const service = materialsService(api);
  if (!service) return rows;
  rows.push({ kind: 'heading', L: layer, key: 'section:materials', label: 'Materials' });
  const time = api.transport.time();
  SURFACES.forEach((surface, index) => {
    const name = SURFACE_NAMES[index]!, open = isOpen(layer, index);
    rows.push({ kind: 'material', L: layer, key: `material:${index}:color`, label: name, index, slot: 'color', entry: true, open, materialName: name } satisfies MaterialRow);
    if (open) rows.push({ kind: 'material', L: layer, key: `material:${index}:emissive`, label: 'Emissive', index, slot: 'emissive', entry: false, open, materialName: name } satisfies MaterialRow);
    let values: Record<string, unknown> | null | undefined;
    for (const [key, label] of SURFACE_OPTIONS) {
      const path = `ms.${surface}.${key}`, prop = api.anim.findProp(layer, path) as any;
      const animated = !!prop && (prop.kf?.length > 0 || !!prop.expr);
      if (!open && !animated) continue;
      if (prop) { rows.push({ kind: 'prop', L: layer, key: path, prop, label, group: name, material: { index, key } }); continue; }
      values ??= service.propertyValues(layer.id, index, time);
      rows.push({ kind: 'prop', L: layer, key: path, prop: { v: values?.[key] ?? 0, kf: [], expr: null }, label, group: name, virtual: { index, key } });
    }
  });
  return rows;
}
