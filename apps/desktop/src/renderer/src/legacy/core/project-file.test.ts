import { describe, expect, it, vi } from 'vitest';
import { packProjectFile, restoreProjectFileMedia, unpackProjectFile, unpackProjectFileBlob, packProjectFileBlob, restoreProjectFileStream, saveIncrementalProject, withProjectReader } from './project-file';
import { sha256Hex } from '../../../../shared/sha256';

describe('portable project media', () => {
  it('uploads only requested media, including media reachable solely from Undo', async () => {
    const old = { id: 'deleted' }, current = { id: 'current' };
    const snapshot = { proj: { assets: { current } }, history: { entries: [{ backward: [{ exists: true, path: ['assets', 'deleted'], value: old }] }] } };
    const serialized = JSON.stringify(snapshot);
    const bridge = { begin: vi.fn(async () => ({ ok: true as const, token: 'token', required: ['deleted'] })),
      chunk: vi.fn(async () => undefined), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => undefined) };
    const store = { get: vi.fn(), put: vi.fn(), getForSave: async (asset: any) => ({ blob: new Blob([asset.id]), revision: `revision-${asset.id}` }) };
    expect(await saveIncrementalProject(snapshot, serialized, store, bridge, { name: 'file.pmv', projectId: 'one', saveAs: false }, () => {})).toMatchObject({ ok: true });
    const chunks = bridge.chunk.mock.calls as unknown as Array<[string, string | null, Uint8Array]>;
    expect(chunks.map(call => call[1])).toEqual([null, 'deleted']);
    expect(new TextDecoder().decode(chunks[0]![2])).toBe(serialized);
    expect(new TextDecoder().decode(chunks[1]![2])).toBe('deleted');
    expect(store.get).not.toHaveBeenCalled();
    expect(bridge.abort).toHaveBeenCalledWith('token');
  });
  it('rejects corrupted streamed media before putting it into the local store', async () => {
    const put = vi.fn(async () => true);
    vi.stubGlobal('navigator', { storage: {} });
    try {
      await expect(restoreProjectFileStream({ proj: { assets: { clip: { id: 'clip' } } } },
        [{ id: 'clip', type: 'video/webm', offset: 0, length: 3, sha256: sha256Hex(new Uint8Array([1, 2, 3])), revision: 'saved' }],
        { get: async () => null, put }, async () => new Uint8Array([4, 5, 6]))).rejects.toThrow('checksum');
      expect(put).not.toHaveBeenCalled();
    } finally { vi.unstubAllGlobals(); }
  });
  it.each(['current', 'undo', 'redo', 'all missing'])('saves with missing %s media while preserving editable source and history', async location => {
    const missing = { id: 'missing', name: '01-window-plate.png', kind: 'image' };
    const available = { id: 'available', name: 'available.png', kind: 'image' };
    const patch = { path: ['assets', missing.id], exists: true, value: missing };
    const document = {
      proj: {
        layers: [{ id: 'image-layer', type: 'image', d: { asset: missing.id } }],
        assets: {
          ...(location === 'current' || location === 'all missing' ? { missing } : {}),
          ...(location !== 'all missing' ? { available } : {}),
        },
      },
      history: { entries: [{
        forward: location === 'redo' ? [patch] : [],
        backward: location === 'undo' ? [patch] : [],
      }], cursor: location === 'redo' ? 0 : 1 },
    };
    const original = JSON.stringify(document);
    const data = new Uint8Array([0, 127, 255]);
    const store = {
      get: async (asset: any) => asset.id === available.id ? new Blob([data], { type: 'image/png' }) : null,
      put: vi.fn(async (_id: string, _blob: Blob, _metadata: any) => true),
    };
    const reopened = unpackProjectFile(await packProjectFile(document, store));
    expect(reopened.proj).toEqual(document.proj);
    expect(reopened.history).toEqual(document.history);
    expect(JSON.stringify(document)).toBe(original);
    await restoreProjectFileMedia(reopened, store);
    expect(store.put).toHaveBeenCalledTimes(location === 'all missing' ? 0 : 1);
    if (location !== 'all missing') {
      const [id, blob, metadata] = store.put.mock.calls[0]!;
      expect(id).toBe(available.id);
      expect(metadata).toEqual(available);
      expect(new Uint8Array(await blob.arrayBuffer())).toEqual(data);
    }
  });

  it('round-trips embedded media beyond the old 256 MiB limit', async () => {
    const chunk = new Uint8Array(1024 * 1024); chunk[0] = 123; chunk[chunk.length - 1] = 45;
    const blob = new Blob(Array(257).fill(chunk), { type: 'video/webm' });
    const document = { proj: { w: 1080, h: 1920, layers: [], assets: { video: { id: 'video' } } } };
    const packed = await packProjectFile(document, { get: async () => blob, put: async () => true });
    const media = unpackProjectFile(packed).containerMedia[0].data;
    expect(media.length).toBe(blob.size);
    expect(media[0]).toBe(123); expect(media[media.length - 1]).toBe(45);
  }, 30000);

  it('stores media as raw PMV3 bytes and restores it losslessly', async () => {
    const original = new Uint8Array([0, 1, 2, 127, 128, 254, 255]);
    const packed = await packProjectFile({ proj: {
      w: 100, h: 100, layers: [], assets: { image: { id: 'image', name: 'image.png' } },
    } }, { get: async () => new Blob([original], { type: 'image/png' }), put: async () => true });
    expect(new TextDecoder().decode(packed.subarray(0, 5))).toBe('PMV3\n');
    expect(packed.byteLength).toBeLessThan(400);
    const unpacked = unpackProjectFile(packed);
    let restored: Blob | undefined;
    const put = vi.fn(async (_id: string, blob: Blob) => { restored = blob; return true; });
    await restoreProjectFileMedia(unpacked, { get: async () => null, put });
    expect(new Uint8Array(await restored!.arrayBuffer())).toEqual(original);
  });

  it('embeds an OBJ asset while keeping expanded geometry out of structured layer source', async () => {
    const source = 'v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n';
    const document = { proj: {
      layers: [{ id: 'mesh', type: 'extension', d: { definition: 'example.obj', data: { assetId: 'obj' }, params: {} } }],
      assets: { obj: { id: 'obj', name: 'triangle.obj', kind: 'model', format: 'obj', triangles: 1 } },
    } };
    const packed = await packProjectFile(document, {
      get: async () => new Blob([source], { type: 'model/obj' }), put: async () => true
    });
    const unpacked = unpackProjectFile(packed);
    expect(unpacked.proj.layers[0].d.data).toEqual({ assetId: 'obj' });
    expect(JSON.stringify(unpacked.proj.layers[0])).not.toContain('positions');
    let restored = '';
    await restoreProjectFileMedia(unpacked, {
      get: async () => null,
      put: async (_id, blob) => { restored = await blob.text(); return true; }
    });
    expect(restored).toBe(source);
  });

  it('restores a video-sized payload without overflowing the regex stack', async () => {
    const put = vi.fn(async (_id: string, _blob: Blob, _metadata: unknown) => true);
    const data = 'AAAA'.repeat(5_000_000);
    await restoreProjectFileMedia({
      proj: { assets: { video: { id: 'video', name: 'video.mp4' } } },
      media: { video: { type: 'video/mp4', data } },
    }, { get: async () => null, put });
    expect(put).toHaveBeenCalledOnce();
    expect(put.mock.calls[0]?.[1].size).toBe(15_000_000);
    expect(put.mock.calls[0]?.[1].type).toBe('video/mp4');
  });

  it.each(['a', 'AA=A', 'AAAA====', '!!!!', 'AAA\n', 'AA==AAAA'])('rejects malformed media before writing: %s', async data => {
    const put = vi.fn(async () => true);
    await expect(restoreProjectFileMedia({
      proj: { assets: { image: { id: 'image' } } },
      media: { image: { type: 'image/png', data } },
    }, { get: async () => null, put })).rejects.toThrow('Invalid saved media');
    expect(put).not.toHaveBeenCalled();
  });

  it('imports browser files through slices without reading the complete container', async () => {
    const packed = await packProjectFileBlob({ proj: { assets: { image: { id: 'image' } } } }, {
      get: async () => new Blob([new Uint8Array([1, 2, 3])], { type: 'image/png' }), put: async () => true
    });
    const wholeRead = vi.spyOn(packed, 'arrayBuffer').mockRejectedValue(new Error('whole-file allocation'));
    const unpacked = await unpackProjectFileBlob(packed);
    let restored: Blob | undefined;
    await restoreProjectFileMedia(unpacked, { get: async () => null, put: async (_id, blob) => { restored = blob; return true; } });
    expect([...new Uint8Array(await restored!.arrayBuffer())]).toEqual([1, 2, 3]);
    expect(wholeRead).not.toHaveBeenCalled();
  });

  it('aborts and removes import staging files when a native media read fails', async () => {
    const writer = { write: vi.fn(), abort: vi.fn(async () => undefined) };
    const root = { getFileHandle: vi.fn(async () => ({ createWritable: async () => writer })), removeEntry: vi.fn(async () => undefined) };
    vi.stubGlobal('navigator', { storage: { getDirectory: async () => root } });
    const put = vi.fn(async () => true);
    try {
      await expect(restoreProjectFileStream({ proj: { assets: { video: { id: 'video' } } } },
        [{ id: 'video', type: 'video/webm', offset: 9, length: 10 }], { get: async () => null, put },
        async () => { throw new Error('read failed'); })).rejects.toThrow('read failed');
      expect(writer.abort).toHaveBeenCalledOnce();
      expect(root.removeEntry).toHaveBeenCalledOnce();
      expect(put).not.toHaveBeenCalled();
    } finally { vi.unstubAllGlobals(); }
  });

  it('keeps legacy project files compatible', async () => {
    const put = vi.fn(async () => true);
    await restoreProjectFileMedia({ proj: { assets: {} } }, { get: async () => null, put });
    expect(put).not.toHaveBeenCalled();
  });
});

describe('linked media and the host media store (section 21)', () => {
  const link = { path: '/Volumes/Footage/A001_C003.mov', hash: 'b'.repeat(64), size: 5 };
  it('saves linked assets as records without bytes and lets main copy stored media', async () => {
    const snapshot = { proj: { requiresEngine: 'native', assets: {
      linked: { id: 'linked', name: 'A001_C003.mov', storage: 'linked', link, type: 'video/quicktime' },
      stored: { id: 'stored', storageKey: 'media:v2:s' },
      page: { id: 'page', storageKey: 'media:v2:p' }
    } } };
    const serialized = JSON.stringify(snapshot);
    const begin = vi.fn(async (_request: any) => ({ ok: true as const, token: 'token', required: ['page'] }));
    const bridge = { begin, chunk: vi.fn(async () => undefined), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => undefined) };
    const store = {
      get: vi.fn(), put: vi.fn(),
      saveSource: async (asset: any) => (asset.id === 'stored' ? { hash: 'c'.repeat(64), size: 7, type: 'image/png', revision: 'c'.repeat(64) } : null),
      getForSave: vi.fn(async (asset: any) => (asset.id === 'page' ? { blob: new Blob(['abc']), revision: 'rev-page' } : null))
    };
    expect(await saveIncrementalProject(snapshot, serialized, store, bridge, { name: 'file.pmv', projectId: 'one', saveAs: false }, () => {})).toMatchObject({ ok: true });
    const request = begin.mock.calls[0]![0];
    expect(request.linked).toEqual([{ id: 'linked', hash: link.hash, size: 5, path: link.path, type: 'video/quicktime' }]);
    expect(request.media).toEqual([
      { id: 'stored', revision: 'c'.repeat(64), type: 'image/png', length: 7, stored: 'c'.repeat(64) },
      { id: 'page', revision: 'rev-page', type: '', length: 3 }
    ]);
    expect((bridge.chunk.mock.calls as unknown as Array<[string, string | null]>).map(call => call[1])).toEqual([null, 'page']);
    expect(store.getForSave).not.toHaveBeenCalledWith(expect.objectContaining({ id: 'linked' }));
  });

  it('uploads stored media from the page when main cannot find it', async () => {
    const snapshot = { proj: { assets: { stored: { id: 'stored', storageKey: 'media:v2:s' } } } };
    const bridge = { begin: vi.fn(async () => ({ ok: true as const, token: 'token', required: ['stored'] })), chunk: vi.fn(async () => undefined),
      finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => undefined) };
    const store = { get: vi.fn(), put: vi.fn(),
      saveSource: async () => ({ hash: 'c'.repeat(64), size: 4, type: '', revision: 'r' }),
      getForSave: async () => ({ blob: new Blob(['wxyz']), revision: 'r' }) };
    expect(await saveIncrementalProject(snapshot, JSON.stringify(snapshot), store, bridge, { name: 'f.pmv', projectId: 'one', saveAs: false }, () => {})).toMatchObject({ ok: true });
    expect((bridge.chunk.mock.calls as unknown as Array<[string, string | null, Uint8Array]>).map(call => [call[1], new TextDecoder().decode(call[2])])).toEqual([[null, JSON.stringify(snapshot)], ['stored', 'wxyz']]);
  });

  it('embeds linked originals in Export › Project and opens as a copied, unstamped project', async () => {
    const snapshot = { proj: { requiresEngine: 'native', layers: [], assets: {
      linked: { id: 'linked', name: 'A001_C003.mov', storageKey: 'media:v2:l', storage: 'linked', link },
      offline: { id: 'offline', name: 'gone.mov', storage: 'linked', link: { ...link, path: '/Gone/gone.mov' } }
    } }, history: { entries: [], index: -1 } };
    const store = { put: vi.fn(), get: async (asset: any) => (asset.id === 'linked' ? new Blob(['12345'], { type: 'video/quicktime' }) : null) };
    const packed = await packProjectFileBlob(snapshot, store);
    const reopened = unpackProjectFile(new Uint8Array(await packed.arrayBuffer()));
    expect(reopened.proj.assets.linked).toEqual({ id: 'linked', name: 'A001_C003.mov', storageKey: 'media:v2:l', contentHash: link.hash, sourcePath: link.path });
    // An original that could not be read stays linked, so the stamp stays.
    expect(reopened.proj.assets.offline.storage).toBe('linked');
    expect(reopened.proj.requiresEngine).toBe('native');
    expect(reopened.containerMedia.map((item: any) => item.id)).toEqual(['linked']);
    const all = await packProjectFileBlob({ ...snapshot, proj: { ...snapshot.proj, assets: { linked: snapshot.proj.assets.linked } } }, store);
    expect(unpackProjectFile(new Uint8Array(await all.arrayBuffer())).proj.requiresEngine).toBeUndefined();
  });
});

describe('opening through main\'s project reader', () => {
  it('keeps an open that read every byte when closing the reader fails', async () => {
    // Main closed the reader itself after 120 s idle while media was still being stored.
    const close = vi.fn(async () => { throw new Error('Unknown project read token'); });
    await expect(withProjectReader(async () => 'restored', close)).resolves.toBe('restored');
    expect(close).toHaveBeenCalledTimes(1);
  });
  it('reports the restore\'s own error, not a failed close', async () => {
    const close = vi.fn(async () => { throw new Error('Unknown project read token'); });
    await expect(withProjectReader(async () => { throw new Error('checksum mismatch'); }, close)).rejects.toThrow('checksum mismatch');
    expect(close).toHaveBeenCalledTimes(1);
  });
  it('closes the reader only after the restore finished', async () => {
    const order: string[] = [];
    await withProjectReader(async () => { await new Promise(resolve => setTimeout(resolve, 5)); order.push('restored'); },
      async () => { order.push('closed'); });
    expect(order).toEqual(['restored', 'closed']);
  });
});
