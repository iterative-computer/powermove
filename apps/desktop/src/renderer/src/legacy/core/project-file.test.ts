import { describe, expect, it, vi } from 'vitest';
import { packProjectFile, readProjectFileDocument, restoreProjectFileMedia, unpackProjectFile, unpackProjectFileBlob, packProjectFileBlob, restoreProjectFileStream, saveIncrementalProject } from './project-file';
import { sha256Hex } from '../../../../shared/sha256';

describe('portable project media', () => {
  it('keeps linked originals out of ordinary saves while portable exports include their full media', async () => {
    const snapshot = { proj: { assets: { clip: { id: 'clip', sourcePath: '/media/clip.mp4' }, paste: { id: 'paste' } } } };
    const store = { get: async (asset: any) => new Blob([asset.id]), put: vi.fn(),
      getForSave: async (asset: any) => asset.id === 'clip' ? { linked: true, revision: 'linked' } : { blob: new Blob(['paste']), revision: 'paste' } };
    const bridge = { begin: vi.fn(async () => ({ ok: true as const, token: 'token', required: ['paste'] })),
      chunk: vi.fn(async () => {}), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => {}) };
    await saveIncrementalProject(snapshot, JSON.stringify(snapshot), store, bridge, { name: 'file.pmv', projectId: 'one', saveAs: false }, () => {});
    expect(bridge.begin).toHaveBeenCalledWith(expect.objectContaining({ media: [expect.objectContaining({ id: 'paste' })] }));
    const portable = unpackProjectFile(await packProjectFile(snapshot, store));
    expect(portable.containerMedia.map((source: any) => source.id)).toEqual(['clip', 'paste']);
  });
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

  it('keeps document reads serial and overlaps only one media read while preserving acknowledged progress and bytes', async () => {
    const limit = 1024 * 1024;
    const bytes = new Uint8Array(2 * limit + 3);
    for (let index = 0; index < bytes.length; index++) bytes[index] = index * 13;
    const media = new Blob([bytes]), empty = new Blob(), skipped = new Blob(['unchanged']);
    const emptySlice = vi.spyOn(empty, 'slice'), skippedSlice = vi.spyOn(skipped, 'slice');
    const assets = { clip: { id: 'clip' }, empty: { id: 'empty' }, skipped: { id: 'skipped' } };
    const document = { proj: { assets }, note: 'x'.repeat(limit) }, serialized = JSON.stringify(document);
    const records = new Map([['clip', media], ['empty', empty], ['skipped', skipped]]);
    const store = { get: vi.fn(), put: vi.fn(), getForSave: async (asset: any) => ({ blob: records.get(asset.id)!, revision: asset.id }) };
    const reads = new Map<number, () => void>();
    let activeReads = 0, maxReads = 0, documentReads = 0;
    const originalSlice = Blob.prototype.slice;
    const documentSlice = vi.spyOn(Blob.prototype, 'slice').mockImplementation(function (this: Blob, start, end, type) {
      documentReads++; return originalSlice.call(this, start, end, type);
    });
    Object.defineProperty(media, 'slice', { value: (start = 0, end?: number) => ({ arrayBuffer: () => {
      activeReads++; maxReads = Math.max(maxReads, activeReads);
      return new Promise<ArrayBuffer>(resolve => reads.set(start, () => {
        activeReads--; resolve(bytes.slice(start, end).buffer);
      }));
    } }) });
    const acknowledgements: Array<() => void> = [];
    let activeWrites = 0, maxWrites = 0;
    const bridge = { begin: vi.fn(async () => ({ ok: true as const, token: 'token', required: ['empty', 'clip'] })),
      chunk: vi.fn((_token: string, _id: string | null, _data: Uint8Array) => {
        activeWrites++; maxWrites = Math.max(maxWrites, activeWrites);
        return new Promise<void>(resolve => acknowledgements.push(() => { activeWrites--; resolve(); }));
      }), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => undefined) };
    const progress = vi.fn();
    const saving = saveIncrementalProject(document, serialized, store, bridge, { name: 'file.pmv', projectId: 'one', saveAs: false }, progress);
    try {
      await vi.waitFor(() => expect(bridge.chunk).toHaveBeenCalledTimes(1));
      expect(documentReads).toBe(1); expect(reads.size).toBe(0); expect(progress.mock.calls).toEqual([[.3]]);
      acknowledgements[0]!();
      await vi.waitFor(() => expect(bridge.chunk).toHaveBeenCalledTimes(2));
      expect(documentReads).toBe(2); expect(reads.size).toBe(0);
      acknowledgements[1]!();
      await vi.waitFor(() => expect(reads.has(0)).toBe(true));
      reads.get(0)!();
      await vi.waitFor(() => expect(bridge.chunk).toHaveBeenCalledTimes(3));
      expect([...reads.keys()]).toEqual([0, limit]);
      const beforeMediaACK = progress.mock.calls.length;
      reads.get(limit)!();
      await Promise.resolve(); await Promise.resolve();
      expect(progress.mock.calls.length).toBe(beforeMediaACK);
      expect(bridge.chunk).toHaveBeenCalledTimes(3);
      expect(reads.has(2 * limit)).toBe(false);
      acknowledgements[2]!();
      await vi.waitFor(() => expect(bridge.chunk).toHaveBeenCalledTimes(4));
      expect(reads.has(2 * limit)).toBe(true);
      reads.get(2 * limit)!(); acknowledgements[3]!();
      await vi.waitFor(() => expect(bridge.chunk).toHaveBeenCalledTimes(5));
      acknowledgements[4]!();
      expect(await saving).toMatchObject({ ok: true });
      expect(maxReads).toBe(1); expect(maxWrites).toBe(1);
      expect(emptySlice).not.toHaveBeenCalled(); expect(skippedSlice).not.toHaveBeenCalled();
      expect(bridge.chunk.mock.calls.map(call => call[1])).toEqual([null, null, 'clip', 'clip', 'clip']);
      const uploaded = bridge.chunk.mock.calls.filter(call => call[1] === 'clip').map(call => call[2]);
      expect(new Uint8Array(await new Blob(uploaded as BlobPart[]).arrayBuffer())).toEqual(bytes);
      const documentBytes = new TextEncoder().encode(serialized).length, total = documentBytes + bytes.length;
      const acknowledged = [limit, documentBytes, documentBytes + limit, documentBytes + 2 * limit, total];
      expect(progress.mock.calls.map(call => call[0])).toEqual([.3, ...acknowledged.map(value => .3 + .6 * value / total)]);
      expect(bridge.abort).toHaveBeenCalledWith('token');
    } finally { documentSlice.mockRestore(); }
  }, 30000);

  it.each(['read', 'write', 'progress'] as const)('drains a speculative media read before aborting after a %s failure', async failure => {
    const limit = 1024 * 1024, media = new Blob([new Uint8Array(2 * limit + 1)]);
    const document = { proj: { assets: { clip: { id: 'clip' } } } };
    let rejectRead!: (error: Error) => void, resolveWrite!: () => void, rejectWrite!: (error: Error) => void;
    const slice = vi.spyOn(media, 'slice').mockImplementation((start = 0) => ({ arrayBuffer: () => start === 0
      ? Promise.resolve(new ArrayBuffer(limit))
      : new Promise<ArrayBuffer>((_resolve, reject) => { rejectRead = reject; }) }) as Blob);
    const readError = new Error('speculative read failed'), primaryError = new Error(`${failure} failed`);
    const bridge = { begin: async () => ({ ok: true as const, token: 'token', required: ['clip'] }),
      chunk: vi.fn(async (_token: string, id: string | null, _data: Uint8Array) => {
        if (id !== null) await new Promise<void>((resolve, reject) => { resolveWrite = resolve; rejectWrite = reject; });
      }), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => undefined) };
    const progress = vi.fn((value: number) => { if (failure === 'progress' && value > .5) throw primaryError; });
    let settled = false;
    const saving = saveIncrementalProject(document, JSON.stringify(document),
      { get: vi.fn(), put: vi.fn(), getForSave: async () => ({ blob: media, revision: 'revision' }) }, bridge,
      { name: 'file.pmv', projectId: 'one', saveAs: false }, progress);
    saving.then(() => { settled = true; }, () => { settled = true; });
    const rejected = expect(saving).rejects.toMatchObject({ cause: failure === 'read' ? readError : primaryError });
    await vi.waitFor(() => expect(bridge.chunk).toHaveBeenCalledTimes(2));
    expect(slice).toHaveBeenCalledTimes(2);
    if (failure === 'read') rejectRead(readError);
    else if (failure === 'write') rejectWrite(primaryError);
    else resolveWrite();
    if (failure === 'progress') await vi.waitFor(() => expect(progress.mock.calls.some(([value]) => value > .5)).toBe(true));
    await Promise.resolve(); await Promise.resolve();
    expect(settled).toBe(false); expect(bridge.abort).not.toHaveBeenCalled();
    if (failure === 'read') resolveWrite(); else rejectRead(readError);
    await rejected;
    expect(bridge.abort).toHaveBeenCalledWith('token'); expect(bridge.finish).not.toHaveBeenCalled();
    expect(bridge.chunk).toHaveBeenCalledTimes(2); expect(slice).toHaveBeenCalledTimes(2);
  });

  it.each(['portable', 'incremental'])('prepares %s project media sequentially without changing file order or missing-media handling', async mode => {
    const document = { proj: { assets: Object.fromEntries(Array.from({ length: 6 }, (_, index) => [`asset-${index}`, { id: `asset-${index}` }])) } };
    const before = JSON.stringify(document);
    const pending = new Map<string, (blob: Blob | null) => void>();
    const read = (asset: any) => new Promise<Blob | null>(resolve => pending.set(asset.id, resolve));
    const store = { get: vi.fn(read), put: vi.fn(), getForSave: vi.fn(async (asset: any) => {
      const blob = await read(asset);
      return blob ? { blob, revision: `revision-${asset.id}` } : null;
    }) };
    const bridge = { begin: vi.fn(async () => ({ ok: true as const, token: 'token', required: [] as string[] })),
      chunk: vi.fn(async () => undefined), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => undefined) };
    const progress = vi.fn();
    const packaging = mode === 'portable'
      ? packProjectFileBlob(document, store, before, progress)
      : saveIncrementalProject(document, before, store, bridge, { name: 'file.pmv', projectId: 'one', saveAs: false }, progress);
    for (let index = 0; index < 6; index++) {
      await vi.waitFor(() => expect(pending.size).toBe(index + 1));
      expect(bridge.begin).not.toHaveBeenCalled();
      pending.get(`asset-${index}`)!(index === 2 ? null : new Blob([`bytes-${index}`], { type: 'image/png' }));
    }
    const result = await packaging;
    const ids = [0, 1, 3, 4, 5].map(index => `asset-${index}`);
    if (mode === 'portable') {
      const unpacked = unpackProjectFile(await (result as Blob).arrayBuffer());
      expect(unpacked.proj).toEqual(document.proj);
      expect(unpacked.containerMedia.map((item: any) => item.id)).toEqual(ids);
      for (const item of unpacked.containerMedia) expect(new TextDecoder().decode(item.data)).toBe(`bytes-${item.id.slice(-1)}`);
      expect(progress.mock.calls.map(call => call[0])).toEqual([1, 2, 3, 4, 5, 6].map(value => value / 6));
      expect(store.getForSave).not.toHaveBeenCalled();
    } else {
      expect(result).toMatchObject({ ok: true });
      expect(bridge.begin).toHaveBeenCalledWith(expect.objectContaining({ media: ids.map(id => ({ id, revision: `revision-${id}`, type: 'image/png', length: 7 })) }));
      expect(store.get).not.toHaveBeenCalled();
      expect(bridge.chunk).toHaveBeenCalledTimes(1);
    }
    expect(JSON.stringify(document)).toBe(before);
  });

  it('settles failed source preparation before rejecting and never begins a native save', async () => {
    const document = { proj: { assets: Object.fromEntries(Array.from({ length: 5 }, (_, index) => [`asset-${index}`, { id: `asset-${index}` }])) } };
    const pending = new Map<string, { resolve: (value: null) => void; reject: (error: Error) => void }>();
    const store = { get: vi.fn(), put: vi.fn(), getForSave: (asset: any) => new Promise<null>((resolve, reject) => pending.set(asset.id, { resolve, reject })) };
    const bridge = { begin: vi.fn(), chunk: vi.fn(), finish: vi.fn(), abort: vi.fn() };
    const saving = saveIncrementalProject(document, JSON.stringify(document), store, bridge, { name: 'file.pmv', projectId: 'one', saveAs: false }, () => {});
    const rejected = expect(saving).rejects.toThrow('source preparation failed');
    await vi.waitFor(() => expect(pending.size).toBe(1));
    pending.get('asset-0')!.reject(new Error('source preparation failed'));
    await rejected;
    expect(pending.has('asset-4')).toBe(false);
    expect(bridge.begin).not.toHaveBeenCalled();
  });

  it.each(['memory', 'OPFS'])('keeps restored media revisions available for range reuse through %s staging', async staging => {
    const data = new Uint8Array([1, 2, 3]);
    const document = { proj: { assets: { clip: { id: 'clip', storageKey: 'media:clip' } } } };
    const source = { id: 'clip', type: 'video/webm', offset: 9, length: data.length, revision: 'saved-revision', sha256: sha256Hex(data) };
    const stored = new Map<string, { blob: Blob; revision: string }>();
    const store = {
      get: async (asset: any) => stored.get(asset.storageKey)?.blob || null,
      put: vi.fn(async (_id: string, blob: Blob, asset: any, revision?: string) => {
        stored.set(asset.storageKey, { blob, revision: revision || 'new-revision' });
        return true;
      }),
      getForSave: async (asset: any) => stored.get(asset.storageKey) || null,
    };
    const parts: Uint8Array[] = [];
    const writer = { write: async (chunk: Uint8Array) => { parts.push(chunk); }, close: async () => undefined };
    const root = { getFileHandle: async () => ({ createWritable: async () => writer, getFile: async () => new Blob(parts as BlobPart[]) }), removeEntry: vi.fn(async () => undefined) };
    vi.stubGlobal('navigator', { storage: staging === 'OPFS' ? { getDirectory: async () => root } : {} });
    try {
      await restoreProjectFileStream(document, [source], store, async (offset, length) => data.subarray(offset - source.offset, offset - source.offset + length));
      expect(store.put).toHaveBeenCalledWith('clip', expect.any(Blob), document.proj.assets.clip, source.revision);
      expect(new Uint8Array(await (await store.get(document.proj.assets.clip))!.arrayBuffer())).toEqual(data);
      const blob = stored.get('media:clip')!.blob;
      const slice = vi.fn(() => { throw new Error('Unchanged media was uploaded again'); });
      Object.defineProperty(blob, 'slice', { value: slice });
      const bridge = {
        begin: vi.fn(async () => ({ ok: true as const, token: 'token', required: [] as string[] })),
        chunk: vi.fn(async () => undefined), finish: async () => ({ ok: true as const, path: 'file.pmv' }), abort: async () => undefined,
      };
      expect(await saveIncrementalProject(document, JSON.stringify(document), store, bridge, { name: 'file.pmv', projectId: 'reopened', saveAs: false }, () => {})).toMatchObject({ ok: true });
      expect(bridge.begin).toHaveBeenCalledWith(expect.objectContaining({ media: [{ id: 'clip', revision: source.revision, type: source.type, length: data.length }] }));
      expect(slice).not.toHaveBeenCalled();
      expect(bridge.chunk.mock.calls.map(call => (call as unknown as [string, string | null])[1])).toEqual([null]);
    } finally { vi.unstubAllGlobals(); }
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
