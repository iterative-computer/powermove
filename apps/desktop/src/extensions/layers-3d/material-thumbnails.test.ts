// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMaterialThumbnails, THUMBNAIL_SIZE } from './material-thumbnails';
import { canAssignLayer, dropLayer, layerDrop, setLayerDropHover, slotAt } from './layer-drop.svelte';

afterEach(() => { vi.useRealTimers(); document.body.replaceChildren(); setLayerDropHover(null); });

function fakeApi(results: Array<{ url: string } | null>) {
  let revision = 1;
  const getMaterialThumbnail = vi.fn(async () => results.length ? results.shift()! : null);
  const api: any = { project: { revision: () => revision }, scene3d: { getMaterialThumbnail } };
  return { api, getMaterialThumbnail, bump: () => { revision++; } };
}
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('material thumbnails', () => {
  it('shows the placeholder until the engine answers, then shares one cached picture', async () => {
    const { api, getMaterialThumbnail } = fakeApi([{ url: 'data:image/png;base64,AAA' }]);
    const thumbnails = createMaterialThumbnails(api);
    const arrived = vi.fn(); thumbnails.subscribe(arrived);
    expect(thumbnails.peek('model', { layerId: 'paint' })).toBeNull();
    expect(getMaterialThumbnail).toHaveBeenCalledWith('model', { layerId: 'paint' }, expect.objectContaining({ size: THUMBNAIL_SIZE }));
    await flush();
    expect(arrived).toHaveBeenCalledOnce();
    expect(thumbnails.peek('model', { layerId: 'paint' })).toBe('data:image/png;base64,AAA');
    expect(getMaterialThumbnail).toHaveBeenCalledOnce();
  });

  it('retries a busy null with a growing pause, keeping the placeholder meanwhile', async () => {
    vi.useFakeTimers();
    const { api, getMaterialThumbnail } = fakeApi([null, null, { url: 'data:ready' }]);
    const thumbnails = createMaterialThumbnails(api);
    expect(thumbnails.peek('model', 'asset-1')).toBeNull();
    await vi.advanceTimersByTimeAsync(0);
    expect(thumbnails.peek('model', 'asset-1')).toBeNull();
    await vi.advanceTimersByTimeAsync(350);
    expect(getMaterialThumbnail).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(700);
    expect(getMaterialThumbnail).toHaveBeenCalledTimes(3);
    expect(thumbnails.peek('model', 'asset-1')).toBe('data:ready');
  });

  it('refreshes after a project edit without flashing back to the placeholder', async () => {
    const { api, getMaterialThumbnail, bump } = fakeApi([{ url: 'data:red' }, { url: 'data:green' }]);
    const thumbnails = createMaterialThumbnails(api);
    thumbnails.peek('model', { compositionId: 'c1' }); await flush();
    bump();
    expect(thumbnails.peek('model', { compositionId: 'c1' })).toBe('data:red');
    await flush();
    expect(getMaterialThumbnail).toHaveBeenCalledTimes(2);
    expect(thumbnails.peek('model', { compositionId: 'c1' })).toBe('data:green');
  });
});

describe('layer drops on material slots', () => {
  function dropApi(check: { ok: true } | { ok: false; reason: string }, layers: Record<string, any> = {}) {
    const setMaterialMap = vi.fn(() => ({ ok: true, message: '' }));
    return {
      scene3d: { canAssignMaterialMap: vi.fn(() => check), setMaterialMap },
      model: { layer: (id: string) => layers[id] ?? null },
      ui: { toast: vi.fn() }, transport: { invalidate: vi.fn() }
    } as any;
  }
  const target = { layerId: 'model', materialIndex: 0, slot: 'color' as const };

  it('uses the engine preflight and words its reasons for people', () => {
    expect(canAssignLayer(dropApi({ ok: true }), target, 'paint')).toEqual({ ok: true });
    expect(canAssignLayer(dropApi({ ok: false, reason: 'Layer material reference would create a cycle' }), target, 'paint'))
      .toEqual({ ok: false, reason: 'That layer already uses this model, so it would loop' });
    expect(canAssignLayer(dropApi({ ok: false, reason: 'Choose an eligible source layer in the same composition' }, { tone: { id: 'tone', type: 'audio' } }), target, 'tone'))
      .toEqual({ ok: false, reason: 'Audio has no picture' });
    expect(canAssignLayer(dropApi({ ok: true }), { ...target, slot: 'normal' }, 'paint')).toEqual({ ok: false, reason: 'Normal takes an image, not a layer' });
    expect(canAssignLayer(dropApi({ ok: true }), target, 'model')).toEqual({ ok: false, reason: 'A model can’t use itself as a material' });
  });

  it('assigns in one undoable edit, and revalidates at drop time', () => {
    const api = dropApi({ ok: true });
    setLayerDropHover({ ...target, sourceId: 'paint', ok: true, reason: null });
    expect(layerDrop.hover).toMatchObject({ layerId: 'model', ok: true });
    expect(dropLayer(api, target, 'paint')).toBe(true);
    expect(layerDrop.hover).toBeNull();
    expect(api.scene3d.setMaterialMap).toHaveBeenCalledOnce();
    expect(api.scene3d.setMaterialMap).toHaveBeenCalledWith('model', 0, 'color', { layerId: 'paint' }, expect.objectContaining({ label: 'Set Base Color' }));
    const refused = dropApi({ ok: false, reason: 'This layer is locked' });
    expect(dropLayer(refused, target, 'paint')).toBe(false);
    expect(refused.scene3d.setMaterialMap).not.toHaveBeenCalled();
    expect(refused.ui.toast).toHaveBeenCalledWith('The model is locked', expect.anything());
  });

  it('finds a Properties slot under the pointer from its address', () => {
    const row = document.createElement('div');
    Object.assign(row.dataset, { materialLayer: 'model', materialSlotIndex: 'all', materialSlotName: 'emissive' });
    const inner = document.createElement('button'); row.append(inner); document.body.append(row);
    document.elementFromPoint = vi.fn(() => inner) as any;
    expect(slotAt(10, 10)).toEqual({ layerId: 'model', materialIndex: null, slot: 'emissive' });
    row.dataset.materialSlotIndex = '1';
    expect(slotAt(10, 10)).toEqual({ layerId: 'model', materialIndex: 1, slot: 'emissive' });
  });
});
