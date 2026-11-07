import type { PowermoveAPI } from 'powermove';

/*
 * The Timeline's Materials group for a 3D model, as in After Effects 27:
 * each material by name with its Base Color source pop-up, and, once the
 * material is opened, its Emissive source and the six keyframeable Material
 * Options. The 3D Layers extension owns the material logic and the picker
 * and shares them as the `materials` service; without it, nothing shows.
 */

type Slot = 'color' | 'emissive' | 'normal' | 'roughness' | 'metalness' | 'ao';
interface PropertySpec { key: string; label: string; kind: 'num' | 'color'; unit?: string }
interface Entry { index: number | null; name: string; maps: Partial<Record<Slot, unknown>> }
export interface MaterialsServiceLike {
  entries(layerId: string): Entry[];
  describe(layerId: string, index: number | null, slot: Slot, map: unknown): { label: string; icon: string | null; status: string };
  usage(sourceId: string): Array<{ layerId: string; name: string; uses: string[] }>;
  usageTitle(uses: Array<{ layerId: string; name: string; uses: string[] }>, hidden: boolean): string;
  openPicker(anchor: { left: number; top: number; bottom: number; width: number }, request: { layerId: string; materialIndex: number | null; slot: Slot }): void;
  properties: readonly PropertySpec[];
  propertyPath(index: number | null, key: string): string;
  propertyValues(layerId: string, index: number | null, time: number): Record<string, unknown> | null;
  installProperty(layerId: string, index: number | null, key: string, value: unknown, keyAt?: number): unknown;
}

export function materialsService(api: Pick<PowermoveAPI, 'services'>): MaterialsServiceLike | null {
  return (api.services?.get?.('materials') as MaterialsServiceLike | null) ?? null;
}

const objectOf = (layer: any) => layer?.d?.data?.object;
export const isMaterialPropKey = (key: unknown): boolean => /^mi\./.test(String(key ?? ''));

/** Only a model with materials gets the group (and so always has a twirl). */
export function hasMaterialRows(api: Pick<PowermoveAPI, 'services'>, layer: any): boolean {
  const service = objectOf(layer) ? materialsService(api) : null;
  return !!service && service.entries(layer.id).length > 0;
}

export interface MaterialRow { kind: 'material'; L: any; key: string; label: string; index: number | null; slot: Slot; entry: boolean; open: boolean; materialName: string }

/** Materials heading, entry rows, and (opened) Emissive plus Material Options.
 * A closed material still lists its animated options, as a layer lists its
 * keyframed properties. Options that still inherit are stand-in rows
 * (`virtual`) until the first edit installs them. */
export function materialSectionRows(api: Pick<PowermoveAPI, 'services' | 'anim' | 'transport'>, layer: any, isOpen: (layer: any, index: number | null) => boolean): any[] {
  const service = objectOf(layer) ? materialsService(api) : null;
  const entries = service?.entries(layer.id) ?? [];
  if (!service || !entries.length) return [];
  const rows: any[] = [{ kind: 'heading', L: layer, key: 'section:materials', label: 'Materials' }];
  const time = api.transport.time();
  for (const entry of entries) {
    const open = isOpen(layer, entry.index), id = entry.index ?? 'all';
    rows.push({ kind: 'material', L: layer, key: `material:${id}:color`, label: entry.name, index: entry.index, slot: 'color', entry: true, open, materialName: entry.name } satisfies MaterialRow);
    if (open) rows.push({ kind: 'material', L: layer, key: `material:${id}:emissive`, label: 'Emissive', index: entry.index, slot: 'emissive', entry: false, open, materialName: entry.name } satisfies MaterialRow);
    let values: Record<string, unknown> | null | undefined;
    for (const spec of service.properties) {
      const path = service.propertyPath(entry.index, spec.key);
      const prop = api.anim.findProp(layer, path) as any;
      const animated = !!prop && (prop.kf?.length > 0 || !!prop.expr);
      if (!open && !animated) continue;
      if (prop) { rows.push({ kind: 'prop', L: layer, key: path, prop, label: spec.label, group: entry.name, material: { index: entry.index, key: spec.key } }); continue; }
      values ??= service.propertyValues(layer.id, entry.index, time);
      rows.push({ kind: 'prop', L: layer, key: path, prop: { v: values?.[spec.key] ?? 0, kf: [], expr: null }, label: spec.label, group: entry.name, virtual: { index: entry.index, key: spec.key } });
    }
  }
  return rows;
}

/** Source layer id → the models that use it as a material, for the row badge.
 * Keyed on the models' map records themselves, so undo, agents and
 * in-place edits all refresh it without a project event. */
let usageCache: { signature: string; map: Map<string, string[]> } | null = null;
export function materialSourceMap(api: Pick<PowermoveAPI, 'project'>): Map<string, string[]> {
  const models = ((api.project.get() as any).layers || []).filter((layer: any) => objectOf(layer));
  const signature = JSON.stringify(models.map((layer: any) => [layer.id, objectOf(layer).material?.maps, objectOf(layer).sourceMaterials]));
  if (usageCache?.signature === signature) return usageCache.map;
  const map = new Map<string, string[]>();
  for (const layer of models) {
    const object = objectOf(layer);
    for (const maps of [object.material?.maps, ...Object.values<any>(object.sourceMaterials || {})]) {
      for (const value of Object.values<any>(maps || {})) {
        if (!value || typeof value !== 'object' || typeof value.layerId !== 'string') continue;
        const receivers = map.get(value.layerId) ?? [];
        if (!receivers.includes(layer.id)) receivers.push(layer.id);
        map.set(value.layerId, receivers);
      }
    }
  }
  usageCache = { signature, map };
  return map;
}
