import type { PowermoveAPI } from 'powermove';
import { assignMap, slotSpec, wrongTypeReason, type MaterialSlot } from './materials';

/*
 * Dragging a layer from the Timeline onto a material slot. The Timeline owns
 * the pointer gesture; while it is over a slot it asks here whether the layer
 * fits (the engine's own preflight, with its reasons) and publishes the hover
 * so the Properties slot can light up. Dropping assigns in one undo step.
 */

export interface SlotRef { layerId: string; materialIndex: number | null; slot: MaterialSlot }
export interface SlotHover extends SlotRef { sourceId: string; ok: boolean; reason: string | null }
export type AssignCheck = { ok: true } | { ok: false; reason: string };

/** The slot a layer drag is over right now, for the Properties highlight. */
export const layerDrop = $state<{ hover: SlotHover | null }>({ hover: null });

export const sameSlot = (a: SlotRef | null | undefined, b: SlotRef | null | undefined): boolean =>
  !!a && !!b && a.layerId === b.layerId && a.materialIndex === b.materialIndex && a.slot === b.slot;

/** A Properties slot under a viewport point (its row carries the slot's address). */
export function slotAt(x: number, y: number): SlotRef | null {
  const element = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-material-layer][data-material-slot-name]');
  if (!element) return null;
  const index = element.dataset.materialSlotIndex;
  return {
    layerId: element.dataset.materialLayer!,
    materialIndex: index === undefined || index === 'all' ? null : Number(index),
    slot: element.dataset.materialSlotName as MaterialSlot
  };
}

/** Plain-language version of the engine's reason, for the drag badge. */
function readable(api: PowermoveAPI, reason: string, slot: MaterialSlot, sourceId: string): string {
  const spec = slotSpec(slot);
  if (/cycle|loop/i.test(reason)) return 'That layer already uses this model, so it would loop';
  if (/base color and emissive/i.test(reason)) return `${spec.label} takes an image, not a layer`;
  if (/locked/i.test(reason)) return 'The model is locked';
  if (/UV coordinates/i.test(reason)) return 'This model has no UVs to wrap a layer around';
  if (/eligible source layer/i.test(reason)) {
    const source = api.model.layer(sourceId);
    return source && ['group', 'precomp', 'solid', 'image', 'video', 'text', 'shape'].includes(source.type) && !(source.d as any)?.data?.object
      ? 'Only layers in this model’s composition can be used'
      : source ? wrongTypeReason(source) : 'That layer is gone';
  }
  return reason.replace(/\.$/, '');
}

export function canAssignLayer(api: PowermoveAPI, target: SlotRef, sourceId: string): AssignCheck {
  if (target.layerId === sourceId) return { ok: false, reason: 'A model can’t use itself as a material' };
  if (!slotSpec(target.slot).layers) return { ok: false, reason: `${slotSpec(target.slot).label} takes an image, not a layer` };
  try {
    const check = api.scene3d.canAssignMaterialMap(target.layerId, target.materialIndex, target.slot, { layerId: sourceId });
    return check.ok ? { ok: true } : { ok: false, reason: readable(api, check.reason, target.slot, sourceId) };
  } catch (error) {
    return { ok: false, reason: error instanceof Error ? error.message : String(error) };
  }
}

export function setLayerDropHover(hover: SlotHover | null): void {
  const current = layerDrop.hover;
  if (!hover && !current) return;
  if (hover && current && sameSlot(hover, current) && hover.sourceId === current.sourceId && hover.ok === current.ok) return;
  layerDrop.hover = hover;
}

/** Revalidates at drop time; the engine's setter shares the preflight. */
export function dropLayer(api: PowermoveAPI, target: SlotRef, sourceId: string): boolean {
  setLayerDropHover(null);
  const check = canAssignLayer(api, target, sourceId);
  if (!check.ok) { api.ui.toast(check.reason, { key: 'material-assign' }); return false; }
  return assignMap(api, target.layerId, target.materialIndex, target.slot, { layerId: sourceId });
}
