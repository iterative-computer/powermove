import { describe, expect, it, vi } from 'vitest';
import { alreadyImported, listMedia, mediaReferences, manageMedia } from './media-management';
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
  it('dedupes agent imports on the full content hash among every asset sharing the fingerprint', async () => {
    const PM = registry();
    const [h1, h2, h3] = ['1', '2', '3'].map(digit => digit.repeat(64));
    PM.proj.assets = {
      a: { id: 'a', fingerprint: 'fp', persisted: true, contentHash: h1 },
      b: { id: 'b', fingerprint: 'fp', persisted: true, contentHash: h2 },
      other: { id: 'other', fingerprint: 'other', persisted: true, contentHash: h3 },
    };
    PM.assets.get = vi.fn(() => ({}));
    let fileHash = h2;
    const hash = vi.fn(async () => fileHash);
    PM.HostMedia = { enabled: true, hash };
    const file = new Blob(['x']);
    // The second asset with the fingerprint is the match, not the first.
    expect((await alreadyImported(PM, PM.proj, 'fp', file))?.id).toBe('b');
    expect(hash).toHaveBeenCalledTimes(1);
    // Other content under the same fingerprint is a new import.
    fileHash = h3;
    expect(await alreadyImported(PM, PM.proj, 'fp', file)).toBeNull();
    // Legacy media without a content hash matches only when its stored bytes are this file's.
    PM.proj.assets.legacy = { id: 'legacy', fingerprint: 'fp', persisted: true };
    const storedBytes: Record<string, Blob> = { legacy: new Blob(['y']) };
    PM.MediaStore = { get: vi.fn(async (item: any) => storedBytes[item.id] ?? null) };
    hash.mockImplementation(async (blob: Blob) => (blob === file ? fileHash : `${await blob.text()}`.repeat(64)));
    fileHash = h3;
    expect(await alreadyImported(PM, PM.proj, 'fp', file)).toBeNull();
    fileHash = 'x'.repeat(64);
    storedBytes.legacy = new Blob(['x']);
    expect((await alreadyImported(PM, PM.proj, 'fp', file))?.id).toBe('legacy');
    // Stored bytes that are a conversion cannot be compared: the fingerprint decides.
    storedBytes.legacy = new Blob(['proxy bytes']);
    PM.proj.assets.legacy.playbackProxy = true;
    expect((await alreadyImported(PM, PM.proj, 'fp', file))?.id).toBe('legacy');
    fileHash = h1;
    expect((await alreadyImported(PM, PM.proj, 'fp', file))?.id).toBe('a');
    // The legacy engine dedupes on the fingerprint alone.
    PM.HostMedia.enabled = false;
    hash.mockClear();
    expect((await alreadyImported(PM, PM.proj, 'fp', file))?.id).toBe('a');
    expect(hash).not.toHaveBeenCalled();
    expect(await alreadyImported(PM, PM.proj, 'none', file)).toBeNull();
  });
});
