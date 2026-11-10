import type { Layer, PowermoveAPI } from 'powermove';

/*
 * Layers as Materials, the way After Effects 27 presents it: a model lists its
 * materials by name, each material slot picks a source (a layer in this comp,
 * a composition or an image) or goes back to the model's Original, and six
 * intrinsic properties can be keyframed per material. Everything here reads
 * the public scene3d API; the inspector, the picker and the timeline share it
 * through the `materials` service so they always agree.
 */

export type MaterialSlot = 'color' | 'normal' | 'roughness' | 'metalness' | 'emissive' | 'ao';
export type MaterialMapValue = string | { compositionId: string } | { layerId: string };
export type MaterialPropertyKey = 'opacity' | 'roughness' | 'metallic' | 'emissionColor' | 'emissionIntensity' | 'ambientResponse';

export interface SlotSpec { id: MaterialSlot; label: string; tip: string; layers: boolean }
/** Base Color and Emissive accept layers and comps; the rest take images only. */
export const MATERIAL_SLOT_SPECS: readonly SlotSpec[] = [
  { id: 'color', label: 'Base Color', layers: true, tip: 'The surface itself. Pick a layer, comp or image to paint it across the whole material.' },
  { id: 'emissive', label: 'Emissive', layers: true, tip: 'Glow. Whatever you pick here lights the surface by itself, even with no lights in the scene.' },
  { id: 'normal', label: 'Normal', layers: false, tip: 'Fine bumps and grooves faked from an image (a normal map). Takes images only.' },
  { id: 'roughness', label: 'Roughness', layers: false, tip: 'An image that says where the surface is glossy (dark) or matte (light). Takes images only.' },
  { id: 'metalness', label: 'Metalness', layers: false, tip: 'An image that says which parts are bare metal (white) and which are not (black). Takes images only.' },
  { id: 'ao', label: 'Occlusion', layers: false, tip: 'Soft contact shadows baked into an image (ambient occlusion). Takes images only.' }
];
export const slotSpec = (slot: MaterialSlot): SlotSpec => MATERIAL_SLOT_SPECS.find(spec => spec.id === slot)!;

export interface PropertySpec { key: MaterialPropertyKey; label: string; kind: 'num' | 'color'; min?: number; max?: number; step?: number; unit?: string; tip: string }
export const MATERIAL_PROPERTY_SPECS: readonly PropertySpec[] = [
  { key: 'opacity', label: 'Opacity', kind: 'num', min: 0, max: 100, step: 1, unit: '%', tip: 'How see-through this material is. 100% is fully solid.' },
  { key: 'roughness', label: 'Roughness', kind: 'num', min: 0, max: 1, step: 0.01, tip: '0 is mirror-smooth and glossy; 1 is soft and matte, like chalk.' },
  { key: 'metallic', label: 'Metallic', kind: 'num', min: 0, max: 1, step: 0.01, tip: '0 is plastic, paint or wood; 1 is bare metal, which tints its reflections.' },
  { key: 'emissionColor', label: 'Emission Color', kind: 'color', tip: 'The color the surface glows, even in the dark.' },
  { key: 'emissionIntensity', label: 'Emission Intensity', kind: 'num', min: 0, max: 100, step: 0.1, tip: 'How brightly it glows. 0 turns the glow off.' },
  { key: 'ambientResponse', label: 'Ambient Response', kind: 'num', min: 0, max: 100, step: 1, unit: '%', tip: 'How much soft, all-around scene light this material picks up. Lower it for deeper shadows; direct lights still work.' }
];

export interface MaterialEntry {
  /** null is the whole model (and the only entry for generated shapes). */
  index: number | null;
  name: string;
  maps: Partial<Record<MaterialSlot, MaterialMapValue>>;
}

const objectOf = (layer: Layer | null | undefined): any => (layer?.d as any)?.data?.object;
export const isModelLayer = (layer: Layer | null | undefined): boolean => !!objectOf(layer);

/** An extruded text or shape layer's Front, Bevel, Side and Back, indexed 0–3 as
 * setMaterialMap takes them, so the picker and the Timeline treat them like materials. */
function surfaceMaterialEntries(layer: Layer | null): MaterialEntry[] | null {
  const extrusion = (layer?.d as any)?.extrusion;
  if ((layer?.type !== 'text' && layer?.type !== 'shape') || !(layer as any).threeD || extrusion?.enabled !== true) return null;
  return (['front', 'bevel', 'side', 'back'] as const).map((surface, index) => ({ index, name: surface[0]!.toUpperCase() + surface.slice(1), maps: { ...(extrusion.surfaces?.[surface]?.maps || {}) } }));
}

export function materialEntries(api: PowermoveAPI, layerId: string): MaterialEntry[] {
  const layer = api.model.layer(layerId), object = objectOf(layer);
  const surfaces = surfaceMaterialEntries(layer);
  if (surfaces) return surfaces;
  if (!object) return [];
  const whole: MaterialMapValue | any = object.material?.maps || {};
  if (!object.source?.assetId) return [{ index: null, name: 'Material', maps: { ...whole } }];
  const materials = api.scene3d.listMaterials(layerId);
  if (!materials.length) return [{ index: null, name: 'Whole Model', maps: { ...whole } }];
  const own = materials.map(material => ({ index: material.index, name: material.name || `Material ${material.index + 1}`, maps: { ...(object.sourceMaterials?.[material.index] || material.maps || {}) } }));
  return materials.length === 1 ? own : [{ index: null, name: 'Whole Model', maps: { ...whole } }, ...own];
}

export const materialKey = (layerId: string, index: number | null): string => `${layerId}:${index ?? 'all'}`;

export type LinkStatus = 'original' | 'ready' | 'missing' | 'cycle' | 'ineligible';
export interface MapDescription { label: string; icon: string | null; status: LinkStatus; kind: 'original' | 'layer' | 'composition' | 'image' }

export function layerIcon(type: string): string {
  return ({ precomp: 'stack', solid: 'solid', image: 'image', video: 'film', text: 'type', shape: 'shape', audio: 'speaker', group: 'layers', adjustment: 'sliders', captions: 'captions' } as Record<string, string>)[type] || 'cube';
}

function compositionName(api: PowermoveAPI, id: string): string | null {
  const project: any = api.project.get();
  if (id === (project.compId || project.id)) return project.compName || project.name || 'Composition';
  return project.comps?.[id]?.name ?? null;
}

export function describeMap(api: PowermoveAPI, layerId: string, index: number | null, slot: MaterialSlot, map: MaterialMapValue | undefined | null): MapDescription {
  const imported = !!objectOf(api.model.layer(layerId))?.source?.assetId;
  if (map == null) return { label: imported ? 'Original' : 'None', icon: null, status: 'original', kind: 'original' };
  if (typeof map === 'string') {
    const asset: any = api.project.get().assets?.[map];
    return asset ? { label: asset.name || 'Image', icon: 'image', status: 'ready', kind: 'image' } : { label: 'Missing image', icon: 'warning', status: 'missing', kind: 'image' };
  }
  if ('compositionId' in map) {
    const name = compositionName(api, map.compositionId);
    return name ? { label: name, icon: 'stack', status: 'ready', kind: 'composition' } : { label: 'Missing comp', icon: 'warning', status: 'missing', kind: 'composition' };
  }
  const source = api.model.layer(map.layerId);
  const link = api.scene3d.materialLinks(layerId).find(item => item.materialIndex === index && item.slot === slot && item.layerId === map.layerId);
  const status: LinkStatus = link?.status ?? (source ? 'ready' : 'missing');
  if (!source || status === 'missing') return { label: 'Missing layer', icon: 'warning', status: 'missing', kind: 'layer' };
  return { label: source.name, icon: layerIcon(source.type), status, kind: 'layer' };
}

/** Plain-language note for a link the engine cannot render; it shows the original instead. */
export function statusNote(status: LinkStatus): string | null {
  if (status === 'missing') return 'The source is gone, so the model shows its original material. Choose another source or Original.';
  if (status === 'cycle') return 'This source now depends on the model itself, which would loop forever, so the original material shows instead.';
  if (status === 'ineligible') return 'This layer can no longer be used as a material, so the original material shows instead.';
  return null;
}

/* ── the source picker's contents ─────────────────────────── */

export type SourceFilter = 'all' | 'comps' | 'footage';
export interface SourceOption {
  id: string;
  label: string;
  icon: string | null;
  map: MaterialMapValue | null;
  detail?: string;
  disabled?: boolean;
  /** Which filter tabs show it ('all' always does). */
  filters: SourceFilter[];
  hidden?: boolean;
}
export interface SourceGroup { id: string; label: string; options: SourceOption[] }

const GROUP_TYPES: Array<{ id: string; label: string; types: string[]; filters: SourceFilter[] }> = [
  { id: 'precomps', label: 'Pre-comp Layers', types: ['precomp'], filters: ['comps'] },
  { id: 'groups', label: 'Groups', types: ['group'], filters: [] },
  { id: 'solids', label: 'Solid Layers', types: ['solid'], filters: [] },
  { id: 'footage', label: 'Footage Layers', types: ['image', 'video'], filters: ['footage'] },
  { id: 'text', label: 'Text & Shape Layers', types: ['text', 'shape'], filters: [] }
];

/** Why a layer of this kind can never be a material. */
export function wrongTypeReason(layer: Layer): string {
  if (isModelLayer(layer) || String((layer.d as any)?.definition ?? '').startsWith('powermove.3d.')) return '3D layers can’t be painted onto a model';
  if (layer.type === 'audio') return 'Audio has no picture';
  if (layer.type === 'adjustment') return 'Adjustment layers have no picture of their own';
  return 'This kind of layer can’t be used as a material';
}

export function sourceGroups(api: PowermoveAPI, layerId: string, slot: MaterialSlot): SourceGroup[] {
  const spec = slotSpec(slot);
  const project: any = api.project.get();
  const imported = !!objectOf(api.model.layer(layerId))?.source?.assetId;
  const groups: SourceGroup[] = [{ id: 'original', label: '', options: [{
    id: 'original', label: imported ? 'Original' : 'None', icon: null, map: null, filters: ['comps', 'footage'],
    detail: imported ? 'The model’s own material from its file' : undefined
  }] }];
  if (spec.layers) {
    const eligible = new Set(api.scene3d.listEligibleSourceLayers(layerId).flatMap(group => group.layers.map(item => item.id)));
    const layers = (project.layers as Layer[]).filter(layer => layer.id !== layerId);
    const known = new Set(GROUP_TYPES.flatMap(group => group.types));
    for (const group of GROUP_TYPES) {
      const options = layers.filter(layer => group.types.includes(layer.type) && !isModelLayer(layer)).map((layer): SourceOption => {
        const ok = eligible.has(layer.id);
        const check=!ok?api.scene3d.canAssignMaterialMap?.(layerId,null,slot,{layerId:layer.id}):null;
        const reason=check?.ok===false&&!/cycle/i.test(check.reason)?check.reason:'Uses this model, so it would loop';
        const hidden = layer.on === false || (layer.on as any)?.v === false;
        return {
          id: `layer:${layer.id}`, label: layer.name, icon: layerIcon(layer.type), map: { layerId: layer.id }, filters: group.filters, hidden,
          disabled: !ok, detail: !ok ? reason : undefined
        };
      });
      groups.push({ id: group.id, label: group.label, options });
    }
    const other = layers.filter(layer => !known.has(layer.type) || isModelLayer(layer));
    groups.push({ id: 'other', label: 'Other Layers', options: other.map(layer => ({
      id: `layer:${layer.id}`, label: layer.name, icon: layerIcon(layer.type), map: null, filters: [], disabled: true, detail: wrongTypeReason(layer)
    })) });
    const valid = new Set(api.scene3d.listValidCompositions(layerId).map(comp => comp.id));
    const comps = [{ id: project.compId || project.id, name: project.compName || project.name }, ...Object.values<any>(project.comps || {}).map(comp => ({ id: comp.id, name: comp.name }))]
      .filter(comp => comp.id);
    groups.push({ id: 'compositions', label: 'Compositions', options: comps.map(comp => ({
      id: `comp:${comp.id}`, label: comp.name || 'Composition', icon: 'stack', map: { compositionId: comp.id }, filters: ['comps'],
      disabled: !valid.has(comp.id), detail: valid.has(comp.id) ? undefined : comp.id === (project.compId || project.id) ? 'This model is in this comp, so it would loop' : 'Contains this model, so it would loop'
    })) });
  }
  const images = Object.entries<any>(project.assets || {}).filter(([, asset]) => asset?.kind === 'image');
  groups.push({ id: 'images', label: 'Images', options: images.map(([id, asset]) => ({ id: `image:${id}`, label: asset.name || 'Image', icon: 'image', map: id, filters: ['footage'] })) });
  return groups;
}

export function filterGroups(groups: SourceGroup[], filter: SourceFilter, query: string): SourceGroup[] {
  const needle = query.trim().toLowerCase();
  return groups.map(group => ({ ...group, options: group.options.filter(option =>
    (filter === 'all' || option.filters.includes(filter)) && (!needle || option.id === 'original' || option.label.toLowerCase().includes(needle))
  ) })).filter(group => group.options.length && !(needle && group.id === 'original'));
}

export const sameMap = (a: MaterialMapValue | null | undefined, b: MaterialMapValue | null | undefined): boolean =>
  JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/* ── "Used as material" ───────────────────────────────────── */

export interface MaterialUse { layerId: string; name: string; uses: string[] }

/** Every model in the open comp that takes this layer as a material, and where. */
export function materialUsage(api: PowermoveAPI, sourceId: string): MaterialUse[] {
  const out: MaterialUse[] = [];
  for (const layer of api.project.get().layers as Layer[]) {
    const object = objectOf(layer);
    const surfaces = surfaceMaterialEntries(layer);
    if (surfaces && layer.id !== sourceId) {
      const uses = surfaces.flatMap(entry => Object.entries<any>(entry.maps).filter(([, map]) => map && typeof map === 'object' && map.layerId === sourceId).map(([slot]) => `${entry.name} · ${slotSpec(slot as MaterialSlot)?.label ?? slot}`));
      if (uses.length) out.push({ layerId: layer.id, name: layer.name, uses });
      continue;
    }
    if (!object || layer.id === sourceId) continue;
    const sets: Array<[number | null, any]> = [[null, object.material?.maps || {}], ...Object.entries<any>(object.sourceMaterials || {}).map(([index, maps]): [number, any] => [Number(index), maps])];
    const uses: string[] = [];
    let names: Map<number, string> | null = null;
    for (const [index, maps] of sets) for (const [slot, map] of Object.entries<any>(maps || {})) {
      if (!map || typeof map !== 'object' || map.layerId !== sourceId) continue;
      if (index != null && !names) names = new Map(api.scene3d.listMaterials(layer.id).map(material => [material.index, material.name]));
      const where = index == null ? (object.source?.assetId ? 'Whole Model' : 'Material') : names?.get(index) || `Material ${index + 1}`;
      uses.push(`${where} · ${slotSpec(slot as MaterialSlot)?.label ?? slot}`);
    }
    if (uses.length) out.push({ layerId: layer.id, name: layer.name, uses });
  }
  return out;
}

export function usageTitle(uses: MaterialUse[], hidden: boolean): string {
  const lines = uses.map(use => `${use.name} (${use.uses.join(', ')})`);
  return `Used as a material — on ${lines.join('; ')}.${hidden ? ' It’s hidden, which is fine: hidden layers still work as materials.' : ''} Click to select ${uses.length === 1 ? uses[0]!.name : 'the models'}.`;
}

/* ── intrinsic properties ─────────────────────────────────── */

export const propertyPath = (index: number | null, key: MaterialPropertyKey): string => `mi.${index == null ? '' : `${index}.`}${key}`;

/** The live channel once installed; null while the property still inherits. */
export function propertyChannel(layer: Layer | null | undefined, index: number | null, key: MaterialPropertyKey): any {
  const object = objectOf(layer);
  return (index == null ? object?.materialProperties?.[key] : object?.sourceMaterialProperties?.[index]?.[key]) ?? null;
}

/** Install a property with its current value (and optionally a key at comp time). One undo step. */
export function installProperty(api: PowermoveAPI, layerId: string, index: number | null, key: MaterialPropertyKey, value: unknown, keyAt?: number) {
  const layer = api.model.layer(layerId);
  const spec = MATERIAL_PROPERTY_SPECS.find(item => item.key === key);
  const patch = keyAt == null ? { [key]: value } : { [key]: { v: value, kf: [{ t: Math.max(0, keyAt - (layer?.from ?? 0)), v: value }], expr: null } };
  return api.scene3d.setMaterialProperties(layerId, index, patch as any, { label: keyAt == null ? `Set ${spec?.label ?? key}` : `Add keyframe for ${spec?.label ?? key}`, origin: 'inspector' });
}

/** Clear every assignment and override on one material, as a single undo step. */
export function resetMaterial(api: PowermoveAPI, layerId: string, index: number | null): boolean {
  const entry = materialEntries(api, layerId).find(item => item.index === index);
  const layer = api.model.layer(layerId);
  const installed = MATERIAL_PROPERTY_SPECS.filter(spec => propertyChannel(layer, index, spec.key));
  if (!entry || (!Object.keys(entry.maps).length && !installed.length)) return false;
  const label = `Reset ${entry.name}`;
  api.edit.begin(label, { origin: 'inspector' });
  try {
    for (const slot of Object.keys(entry.maps) as MaterialSlot[]) {
      const result = api.scene3d.setMaterialMap(layerId, index, slot, null, { label });
      if (!result.ok) throw new Error(result.message);
    }
    if (installed.length) {
      const result = api.scene3d.setMaterialProperties(layerId, index, Object.fromEntries(installed.map(spec => [spec.key, null])) as any, { label });
      if (!result.ok) throw new Error(result.message);
    }
    api.edit.commit(label);
    return true;
  } catch (error) {
    api.edit.cancel();
    api.ui.toast(error instanceof Error ? error.message : String(error), { error: true });
    return false;
  }
}

export function overrideCount(api: PowermoveAPI, layerId: string, entry: MaterialEntry): number {
  const layer = api.model.layer(layerId);
  return Object.keys(entry.maps).length + MATERIAL_PROPERTY_SPECS.filter(spec => propertyChannel(layer, entry.index, spec.key)).length;
}

/** Assign (or clear) one slot, reporting the engine's reason gently if it refuses. */
export function assignMap(api: PowermoveAPI, layerId: string, index: number | null, slot: MaterialSlot, map: MaterialMapValue | null): boolean {
  const spec = slotSpec(slot);
  const result = api.scene3d.setMaterialMap(layerId, index, slot, map, { label: map == null ? `Restore ${spec.label}` : `Set ${spec.label}`, origin: 'inspector' });
  if (!result.ok) {
    const message = /cycle/i.test(result.message)
      ? 'That would loop: the source already uses this model. Pick another layer, or pre-compose it separately.'
      : result.message;
    api.ui.toast(message, { error: true, key: 'material-assign' });
    return false;
  }
  api.transport.invalidate();
  return true;
}
