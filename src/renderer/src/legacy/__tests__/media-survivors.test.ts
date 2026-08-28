import { webcrypto } from 'node:crypto';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { makePM } from './make-pm';

function fakeIndexedDB(abortWrite=false) {
  const records = new Map<string, unknown>();
  const request = (result: unknown, tx: any) => {
    const req: any = { result };
    setImmediate(() => {
      req.onsuccess?.();
      setImmediate(() => abortWrite && tx.mode==='readwrite' ? tx.onabort?.() : tx.oncomplete?.());
    });
    return req;
  };
  const db: any = {
    objectStoreNames: { contains: () => true },
    createObjectStore() {},
    transaction(_store:string,mode:string) {
      const tx: any = {mode};
      tx.objectStore = () => ({
        put(record: any) { records.set(record.id, record); return request(record.id, tx); },
        get(id: string) { return request(records.get(id), tx); },
        delete(id: string) { records.delete(id); return request(undefined, tx); },
      });
      return tx;
    },
  };
  return {
    open() {
      const req: any = { result: db };
      setImmediate(() => req.onsuccess?.());
      return req;
    },
  };
}

function mediaPM(indexedDB: unknown = null) {
  vi.stubGlobal('window', { indexedDB, crypto: webcrypto, addEventListener() {} });
  return makePM('core/media');
}

afterEach(() => vi.unstubAllGlobals());

describe('media oracle survivors', () => {
  it('does not report a request as durable when its transaction later aborts',async()=>{
    const PM=mediaPM(fakeIndexedDB(true));
    expect(await PM.MediaStore.put('aborted',new Blob(['bytes']))).toBe(false);
  });
  it('round-trips generic imported blobs through MediaStore', async () => {
    const PM = mediaPM(fakeIndexedDB());
    const original = new Blob(['exact media bytes'], { type: 'video/mp4' });

    expect(await PM.MediaStore.put('media-1', original)).toBe(true);
    const restored = await PM.MediaStore.get('media-1');

    expect(restored).toBeInstanceOf(Blob);
    expect(restored.type).toBe('video/mp4');
    expect(await restored.text()).toBe('exact media bytes');
  });

  it('uses content rather than file names and MIME metadata for fingerprints', async () => {
    const PM = mediaPM();
    const first: any = new Blob(['same media bytes'], { type: 'audio/mpeg' });
    const renamed: any = new Blob(['same media bytes'], { type: 'application/octet-stream' });
    first.name = 'first.mp3';
    renamed.name = 'renamed.mp3';

    expect(await PM.MediaImport.fingerprint(first)).toBe(await PM.MediaImport.fingerprint(renamed));
  });

  it('reads at most three bounded samples when fingerprinting huge media', async () => {
    const PM = mediaPM();
    const reads: Array<[number, number]> = [];
    const huge = {
      size: 8 * 1024 * 1024 * 1024,
      type: 'video/mp4',
      slice(start: number, end: number) {
        reads.push([start, end]);
        return { arrayBuffer: async () => new Uint8Array(end - start).buffer };
      },
    };

    expect(await PM.MediaImport.fingerprint(huge)).toMatch(/^v2:/);
    expect(reads.length).toBeLessThanOrEqual(3);
    expect(reads.reduce((sum, [start, end]) => sum + end - start, 0)).toBeLessThanOrEqual(192 * 1024);
  });

  it('coalesces legacy duplicate imports onto the live asset', () => {
    const PM = mediaPM();
    const project: any = {
      assets: {
        missing: { id: 'missing', name: 'song.mp3', kind: 'audio', dur: 29.58 },
        live: { id: 'live', name: 'song.mp3', kind: 'audio', dur: 29.58, size: 451000 },
      },
      layers: [{ id: 'audio-layer', type: 'audio', d: { asset: 'missing' } }],
      comps: {},
    };
    const liveAssets = new Map([['live', { id: 'live' }]]);
    const identity = {
      name: 'song.mp3', kind: 'audio', dur: 29.58, size: 451000, fingerprint: 'v2:451000:abc',
    };

    const plan = PM.MediaImport.match(project, liveAssets, identity);
    expect(plan).toEqual({ canonicalId: 'live', aliases: ['missing'] });
    expect(PM.MediaImport.coalesce(project, plan.canonicalId, plan.aliases)).toBe(1);
    expect(project.layers[0].d.asset).toBe('live');
    expect(project.assets.missing).toBeUndefined();
  });

  it('removes all project references to deleted media and safely unparents children', () => {
    const PM = mediaPM();
    const project: any = {
      assets: {
        used: { id: 'used', name: 'clip.mov', kind: 'video' },
        kept: { id: 'kept', name: 'still.png', kind: 'image' },
      },
      layers: [
        { id: 'parent', type: 'video', d: { asset: 'used' } },
        { id: 'child', parent: 'parent', type: 'text', d: {} },
        { id: 'kept-layer', type: 'image', d: { asset: 'kept' } },
      ],
      comps: { nested: { layers: [{ id: 'nested-use', type: 'video', d: { asset: 'used' } }] } },
    };

    expect(PM.MediaImport.referenceCount(project, 'used')).toBe(2);
    const result = PM.MediaImport.removeAsset(project, 'used');

    expect(result.removedLayers).toBe(2);
    expect(result.removedLayerIds.sort()).toEqual(['nested-use', 'parent']);
    expect(project.assets.used).toBeUndefined();
    expect(project.assets.kept).toBeTruthy();
    expect(project.layers.map((layer: any) => layer.id)).toEqual(['child', 'kept-layer']);
    expect(project.layers[0].parent).toBeNull();
    expect(project.comps.nested.layers).toEqual([]);
  });

  it('splits audio without losing source continuity or sharing content objects', () => {
    const PM = mediaPM();
    // Run the real shortcut installer after the media installer, matching bootstrap.
    const shortcuts = makePM('core/media', 'ui/shortcuts');
    Object.assign(shortcuts, PM);
    const layer: any = {
      id: 'audio-1', type: 'audio', from: 2, dur: 10, d: { asset: 'asset-1', trim: 1 },
    };
    shortcuts.proj = { w: 1920, h: 1080, fps: 30, dur: 8, assets: {}, layers: [layer] };
    shortcuts.time = 5;
    shortcuts.hist = { do: (_label: string, action: () => unknown) => action() };
    shortcuts.selLayers = () => [layer];
    shortcuts.cloneLayer = (value: unknown) => JSON.parse(JSON.stringify(value));
    shortcuts.bus = { emit() {} };

    shortcuts.cmd('split');

    expect(layer.dur).toBe(3);
    const right = shortcuts.proj.layers[0];
    expect({ from: right.from, dur: right.dur, trim: right.d.trim }).toEqual({ from: 5, dur: 7, trim: 4 });
    expect(layer.d.trim).toBe(1);
    expect(right.d).not.toBe(layer.d);
  });
});
