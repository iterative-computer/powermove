import { describe, expect, it, vi } from 'vitest';
import { packProjectFile, restoreProjectFileMedia, unpackProjectFile, unpackProjectFileBlob, packProjectFileBlob, restoreProjectFileStream } from './project-file';

describe('portable project media', () => {
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
