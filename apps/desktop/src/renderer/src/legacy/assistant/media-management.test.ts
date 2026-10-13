import { describe, expect, it, vi } from 'vitest';
import { listMedia, mediaReferences, manageMedia } from './media-management';
import { install } from '../core/media';

function registry() {
  const PM: any = { proj: { id: 'p', revision: 0, assets: { a: { id: 'a', name: 'Image', kind: 'image' }, b: { id: 'b', name: 'Audio', kind: 'audio' } }, layers: [], comps: {} }, assets: { get: vi.fn(), loading: new Set(), errors: new Map() }, sel: { layers: [] }, bus: { emit: vi.fn() }, hist: { do: vi.fn((_label, fn) => fn()) } };
  install(PM);
  return PM;
}

describe('agent media management', () => {
  it('reports status, bounded pages and references inside nested compositions and locked groups', () => {
    const PM = registry();
    PM.proj.comps.child = { id: 'child', layers: [{ id: 'g', type: 'group', lock: true }, { id: 'model', group: 'g', d: { data: { object: { material: { maps: { color: 'a' } } } } } }] };
    PM.assets.loading.add('b');
    expect(listMedia(PM, { limit: 1 })).toMatchObject({ total: 2, nextOffset: 1, assets: [{ id: 'a', status: 'missing', lockedReferences: 1, referenceCount: 1 }] });
    expect(listMedia(PM, { missingOnly: true }).assets.map(asset => asset.id)).toEqual(['a']);
    expect(mediaReferences(PM.proj, 'a')[0]).toMatchObject({ compositionId: 'child', locked: true });
    expect(() => manageMedia(PM, { operation: 'delete', assetIds: ['a'], removeReferenced: true })).toThrow('Unlock');
    expect(PM.hist.do).not.toHaveBeenCalled();
  });
  it('rejects referenced deletion without explicit intent and rejects unsupported structured references', () => {
    const PM = registry();
    PM.proj.layers = [{ id: 'image', type: 'image', d: { asset: 'a' } }];
    expect(() => manageMedia(PM, { operation: 'delete', assetIds: ['a'] })).toThrow('removeReferenced');
    PM.proj.layers = [{ id: 'material', d: { shader: { image: { assetId: 'a' } } } }];
    expect(() => manageMedia(PM, { operation: 'delete', assetIds: ['a'], removeReferenced: true })).toThrow('structured references');
    expect(PM.hist.do).not.toHaveBeenCalled();
    expect(PM.proj.assets.a).toBeDefined();
  });
  it('deletes referenced layers only with explicit intent, and rejects invalid batches before mutations', () => {
    const PM = registry();
    PM.proj.layers = [{ id: 'image', type: 'image', d: { asset: 'a' } }];
    PM.sel.layers = ['image'];
    expect(() => manageMedia(PM, { operation: 'delete', assetIds: ['a', 'unknown'], removeReferenced: true })).toThrow('existing assetIds');
    expect(PM.hist.do).not.toHaveBeenCalled();
    expect(manageMedia(PM, { operation: 'delete', assetIds: ['a'], removeReferenced: true }).removedLayerIds).toEqual(['image']);
    expect(PM.proj.assets.a).toBeUndefined();
    expect(PM.proj.layers).toEqual([]);
    expect(PM.sel.layers).toEqual([]);
  });
});
