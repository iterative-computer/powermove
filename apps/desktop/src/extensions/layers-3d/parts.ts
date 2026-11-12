import type { Layer, PowermoveAPI } from 'powermove';

/* Helpers for the Parts section: a model's own pieces, edited in place. */
export type PartField = 'x' | 'y' | 'z' | 'rx' | 'ry' | 'rz' | 'scale' | 'visible';
const NEUTRAL: Record<PartField, unknown> = { x: 0, y: 0, z: 0, rx: 0, ry: 0, rz: 0, scale: 100, visible: true };

const partData = (layer: Layer | null | undefined, key: string): any => (layer?.d as any)?.data?.object?.parts?.[key] ?? null;
export function partChannel(layer: Layer | null | undefined, key: string, field: PartField): any {
  return partData(layer, key)?.p?.[field] ?? null;
}
/** Moved, turned, scaled, hidden or animated: anything Reset would undo. */
export function partEdited(layer: Layer | null | undefined, key: string): boolean {
  const part = partData(layer, key);
  return !!part && (!!part.matrix?.some((value:number,index:number)=>Math.abs(value-(index%5===0?1:0))>1e-9)
    || Object.entries<any>(part.p || {}).some(([field, channel]) => channel?.kf?.length || channel?.expr || channel?.v !== NEUTRAL[field as PartField]));
}
export function installPart(api: PowermoveAPI, layerId: string, key: string): void {
  const result = api.scene3d.setPart(layerId, key, {}, { label: 'Edit model part', origin: 'inspector' });
  if (!result.ok) api.ui.toast(result.message, { error: true });
}
export function resetPart(api: PowermoveAPI, layerId: string, key: string, name: string): void {
  const result = api.scene3d.setPart(layerId, key, null, { label: `Reset ${name}`, origin: 'inspector' });
  if (!result.ok) api.ui.toast(result.message, { error: true });
  api.transport.invalidate();
}
export function togglePartVisible(api: PowermoveAPI, layerId: string, key: string, name: string): void {
  const visible = api.scene3d.getPart(layerId, key).values.visible;
  const result = api.scene3d.setPart(layerId, key, { visible: !visible }, { label: `${visible ? 'Hide' : 'Show'} ${name}`, origin: 'inspector' });
  if (!result.ok) api.ui.toast(result.message, { error: true });
  api.transport.invalidate();
}
