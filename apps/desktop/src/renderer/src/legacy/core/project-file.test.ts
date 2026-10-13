import { describe, expect, it, vi } from 'vitest';
import { packProjectFile, readProjectFileDocument, restoreProjectFileMedia, unpackProjectFile, unpackProjectFileBlob, packProjectFileBlob, restoreProjectFileStream, saveIncrementalProject } from './project-file';
import { sha256Hex } from '../../../../shared/sha256';

describe('portable project media', () => {
  it('assembles a large document in bounded reads without splitting Unicode or losing Undo records', async () => {
    const prefix = '{"proj":{"w":1920,"h":1080,"layers":[],"assets":{},"note":"';
    const note = 'x'.repeat(1024 * 1024 - prefix.length - 1) + '🟢 Café';
    const json = prefix + note + '"},"history":{"version":1,"index":0,"entries":[{"label":"Undo ☃",'
      + '"forward":[{"path":["name"],"exists":true,"value":"Après"}],"backward":[]}]}}';
    const data = new TextEncoder().encode(json), offset = 42;
    expect(data.indexOf(0xf0)).toBe(1024 * 1024 - 1);
    const read = vi.fn(async (at: number, length: number) => data.subarray(at - offset, at - offset + length));
    const progress = vi.fn();
    const document = await readProjectFileDocument({ offset, length: data.length, sha256: sha256Hex(data) }, offset + data.length, read, progress);
    expect(document).toEqual(JSON.parse(json));
    expect(document.proj.note).toBe(note);
    expect(read.mock.calls).toEqual([[offset, 1024 * 1024], [offset + 1024 * 1024, data.length - 1024 * 1024]]);
    expect(progress).toHaveBeenLastCalledWith(data.length, data.length);
  });

  it('handles short document reads and rejects truncation or altered content', async () => {
    const data = new TextEncoder().encode('{"proj":{"layers":[]},"history":{"entries":[]}}');
    const range = { offset: 0, length: data.length, sha256: sha256Hex(data) };
    expect(await readProjectFileDocument(range, data.length, async (offset, length) => data.subarray(offset, offset + Math.min(3, length))))
      .toEqual(JSON.parse(new TextDecoder().decode(data)));
    await expect(readProjectFileDocument(range, data.length, async () => new Uint8Array())).rejects.toThrow('truncated');
    const changed = data.slice(); changed[1] = changed[1]! ^ 1;
    await expect(readProjectFileDocument(range, data.length, async () => changed)).rejects.toThrow('checksum');
    await expect(readProjectFileDocument(range, data.length, async () => new Uint8Array(data.length + 1))).rejects.toThrow('truncated');
  });

  it.each([{ offset: -1 }, { length: 0 }, { offset: Number.MAX_SAFE_INTEGER }, { sha256: 'invalid' }])
    ('rejects an invalid document range before requesting file bytes: %j', async override => {
      const read = vi.fn();
      await expect(readProjectFileDocument({ offset: 0, length: 2, sha256: 'a'.repeat(64), ...override }, 20, read)).rejects.toThrow('range');
      expect(read).not.toHaveBeenCalled();
    });

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

  it('preserves a corrupt-media error when removing its import staging file also fails', async () => {
    const writer = { write: vi.fn(async () => undefined), close: vi.fn(async () => undefined), abort: vi.fn(async () => undefined) };
    const root = { getFileHandle: vi.fn(async () => ({ createWritable: async () => writer })),
      removeEntry: vi.fn(async () => { throw new Error('Could not remove staging file.'); }) };
    vi.stubGlobal('navigator', { storage: { getDirectory: async () => root } });
    const put = vi.fn(async () => true);
    try {
      await expect(restoreProjectFileStream({ proj: { assets: { video: { id: 'video' } } } },
        [{ id: 'video', type: 'video/webm', offset: 9, length: 3, sha256: sha256Hex(new Uint8Array([1, 2, 3])) }],
        { get: async () => null, put }, async () => new Uint8Array([4, 5, 6]))).rejects.toThrow('project media checksum');
      expect(writer.close).toHaveBeenCalledOnce();
      expect(root.removeEntry).toHaveBeenCalledOnce();
      expect(put).not.toHaveBeenCalled();
    } finally { vi.unstubAllGlobals(); }
  });

  it('still removes staging and preserves the read error when abort throws synchronously', async () => {
    const writer = { write: vi.fn(), abort: vi.fn(() => { throw new Error('Writer abort failed.'); }) };
    const root = { getFileHandle: vi.fn(async () => ({ createWritable: async () => writer })), removeEntry: vi.fn(async () => undefined) };
    vi.stubGlobal('navigator', { storage: { getDirectory: async () => root } });
    try {
      await expect(restoreProjectFileStream({ proj: { assets: { video: { id: 'video' } } } },
        [{ id: 'video', type: 'video/webm', offset: 9, length: 3 }], { get: async () => null, put: vi.fn() },
        async () => { throw new Error('Native project media read failed.'); })).rejects.toThrow('Native project media read failed.');
      expect(writer.abort).toHaveBeenCalledOnce();
      expect(root.removeEntry).toHaveBeenCalledOnce();
    } finally { vi.unstubAllGlobals(); }
  });

  it('reports staging removal errors when the media import itself succeeded', async () => {
    const data = new Uint8Array([1, 2, 3]);
    const writer = { write: vi.fn(async () => undefined), close: vi.fn(async () => undefined) };
    const root = { getFileHandle: vi.fn(async () => ({ createWritable: async () => writer, getFile: async () => new Blob([data]) })),
      removeEntry: vi.fn(async () => { throw new Error('Could not remove staging file.'); }) };
    vi.stubGlobal('navigator', { storage: { getDirectory: async () => root } });
    const put = vi.fn(async () => true);
    try {
      await expect(restoreProjectFileStream({ proj: { assets: { video: { id: 'video' } } } },
        [{ id: 'video', type: 'video/webm', offset: 9, length: data.length }],
        { get: async () => null, put }, async () => data)).rejects.toThrow('Could not remove staging file.');
      expect(put).toHaveBeenCalledOnce();
      expect(root.removeEntry).toHaveBeenCalledOnce();
    } finally { vi.unstubAllGlobals(); }
  });

  it('keeps legacy project files compatible', async () => {
    const put = vi.fn(async () => true);
    await restoreProjectFileMedia({ proj: { assets: {} } }, { get: async () => null, put });
    expect(put).not.toHaveBeenCalled();
  });
});
