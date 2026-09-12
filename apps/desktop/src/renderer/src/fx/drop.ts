/* FX browser drag payload (see preview-contract.md, "Drag payload"). Lane B
   sets it on the card; the viewer and timeline read it here. */

export const FX_DRAG_MIME = 'application/x-powermove-fx';

export type FxDragPayload = { kind: 'effect' | 'transition'; id: string; label: string };

/** True when a dragover/drop event carries an FX payload (data is unreadable during dragover; types are). */
export function hasFxDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt) return false;
  return Array.from(dt.types || []).includes(FX_DRAG_MIME);
}

/** Parse the FX payload off a drop event; null when absent or malformed. */
export function readFxDrag(dt: DataTransfer | null | undefined): FxDragPayload | null {
  if (!dt) return null;
  let raw = '';
  try { raw = dt.getData(FX_DRAG_MIME); } catch { return null; }
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object') return null;
    if (parsed.kind !== 'effect' && parsed.kind !== 'transition') return null;
    if (typeof parsed.id !== 'string' || !parsed.id) return null;
    return { kind: parsed.kind, id: parsed.id, label: typeof parsed.label === 'string' && parsed.label ? parsed.label : parsed.id };
  } catch {
    return null;
  }
}

/* ── Media drags ──────────────────────────────────────────
   Two sources: asset cards dragged out of the Media panel (carry an asset
   payload) and OS files (carry `Files`). The Media panel and the timeline
   both accept either; the window-level handler in app.ts is the fallback. */

export const ASSET_DRAG_MIME = 'application/x-powermove-asset';

export type AssetDragPayload = { id: string; name: string; kind: string; dur?: number };

export function hasAssetDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt) return false;
  return Array.from(dt.types || []).includes(ASSET_DRAG_MIME);
}

export function hasFileDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt) return false;
  return Array.from(dt.types || []).includes('Files');
}

/** True for any drag the media drop targets care about (asset card or OS files). */
export function hasMediaDrag(dt: DataTransfer | null | undefined): boolean {
  return hasAssetDrag(dt) || hasFileDrag(dt);
}

export function writeAssetDrag(dt: DataTransfer | null | undefined, payload: AssetDragPayload): void {
  if (!dt) return;
  dt.setData(ASSET_DRAG_MIME, JSON.stringify(payload));
  dt.effectAllowed = 'copy';
}

export function readAssetDrag(dt: DataTransfer | null | undefined): AssetDragPayload | null {
  if (!dt) return null;
  let raw = '';
  try { raw = dt.getData(ASSET_DRAG_MIME); } catch { return null; }
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || typeof parsed.id !== 'string' || !parsed.id) return null;
    return {
      id: parsed.id,
      name: typeof parsed.name === 'string' && parsed.name ? parsed.name : 'Media',
      kind: typeof parsed.kind === 'string' ? parsed.kind : 'image',
      dur: Number(parsed.dur) > 0 ? Number(parsed.dur) : undefined,
    };
  } catch {
    return null;
  }
}

/** Apply a dropped effect/transition to a layer through the edit pipeline. Returns true on success. */
export function applyFxDrop(payload: FxDragPayload, layerId: string | null | undefined, edge?: 'in' | 'out', pm: any = (globalThis as any).PM): boolean {
  const PM = pm;
  if (!PM) return false;
  const toast = (text: string) => PM.toast?.(text);
  const layer = layerId ? PM.L?.(layerId) ?? PM.proj?.layers?.find((l: any) => l.id === layerId) : null;
  if (!layer) { toast('Select a layer to apply ' + payload.label); return false; }
  const meta = PM.TYPE_META?.[layer.type];
  if (layer.type === 'audio' || meta?.effects === false) {
    toast(`${meta?.label || 'Audio'} layers do not support ${payload.kind === 'effect' ? 'effects' : 'transitions'}`);
    return false;
  }
  try {
    const side = edge || 'in';
    const result = payload.kind === 'effect'
      ? PM.Edit.apply(
        { type: 'add_effect', target: layer.id, effect: payload.id },
        { label: 'Add ' + payload.label, origin: 'fx-browser' }
      )
      : PM.Edit.apply(
        { type: 'set_transition', layer: layer.id, edge: side, transition: { type: payload.id } },
        { label: 'Set ' + payload.label, origin: 'fx-browser' }
      );
    if (result && result.ok === false) {
      toast(result.message || 'Could not apply ' + payload.label);
      return false;
    }
    toast(payload.kind === 'effect'
      ? `Added ${payload.label} to ${layer.name}`
      : `Added ${payload.label} (${side}) to ${layer.name}`);
    PM.invalidate?.();
    return true;
  } catch (error: any) {
    toast(error?.message || 'Could not apply ' + payload.label);
    return false;
  }
}
