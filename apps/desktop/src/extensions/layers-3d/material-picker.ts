import { mount, unmount } from 'svelte';
import type { PowermoveAPI } from 'powermove';
import MaterialPicker from './MaterialPicker.svelte';
import { assignMap, materialEntries, sourceGroups, type MaterialMapValue, type MaterialSlot } from './materials';

export interface PickerAnchor { left: number; top: number; bottom: number; width: number }
export interface PickerRequest { layerId: string; materialIndex: number | null; slot: MaterialSlot }

let open: { close(): void; key: string } | null = null;
// A press on the trigger dismisses the sheet before its click arrives; that
// click must not reopen it, so a toggle reads as a toggle.
let lastClosed = { key: '', press: -1 };
let presses = 0;
let counting = false;
const countPress = () => { presses++; };

export function closeMaterialPicker(): void { open?.close(); }
export function materialPickerOpen(): boolean { return !!open; }

/** One picker at a time; activating the same trigger again closes it. */
export function openMaterialPicker(api: PowermoveAPI, anchor: HTMLElement | PickerAnchor, request: PickerRequest): void {
  const key = `${request.layerId}:${request.materialIndex ?? 'all'}:${request.slot}`;
  if (open) {
    const same = open.key === key;
    open.close();
    if (same) return;
  }
  // Same trigger, same press that just dismissed it: that was a toggle.
  if (lastClosed.key === key && lastClosed.press === presses) return;
  if (!counting) { counting = true; document.addEventListener('pointerdown', countPress, true); }
  const entry = materialEntries(api, request.layerId).find(item => item.index === request.materialIndex);
  if (!entry) return;
  const box = anchor instanceof HTMLElement ? anchor.getBoundingClientRect() : anchor;
  const element = anchor instanceof HTMLElement ? anchor : null;
  element?.setAttribute('aria-expanded', 'true');
  const target = document.createElement('div');
  document.body.append(target);
  let closed = false;
  const close = (outside = false) => {
    if (closed) return;
    closed = true;
    open = null;
    lastClosed = outside ? { key, press: presses } : { key: '', press: -1 };
    element?.setAttribute('aria-expanded', 'false');
    void unmount(component);
    target.remove();
  };
  const component = mount(MaterialPicker, {
    target,
    props: {
      api,
      layerId: request.layerId,
      rect: { left: box.left, top: box.top, bottom: box.bottom, width: box.width },
      slot: request.slot,
      materialName: entry.name,
      groups: sourceGroups(api, request.layerId, request.slot),
      current: entry.maps[request.slot] ?? null,
      onPick: (map: MaterialMapValue | null) => { assignMap(api, request.layerId, request.materialIndex, request.slot, map); },
      onImage: () => { void chooseImage(api, request); },
      onClose: (outside?: boolean) => { close(outside); if (!outside) element?.focus({ preventScroll: true }); }
    }
  });
  open = { close, key };
}

export async function chooseImage(api: PowermoveAPI, request: PickerRequest): Promise<void> {
  const [file] = await api.assets.pick({ accept: 'image/*' });
  if (!file) return;
  const asset = await api.assets.import(file);
  assignMap(api, request.layerId, request.materialIndex, request.slot, asset.id);
}
