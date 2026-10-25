// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './media';

function mediaRegistry(): PMRegistry {
  const PM: PMRegistry = {};
  install(PM);
  return PM;
}

describe('legacy media install', () => {
  it('derives poster storage next to its media bytes', () => {
    const PM = mediaRegistry();

    expect(PM.MediaImport.posterKeyFor('media:v2:123:abc')).toBe('media:v2:123:abc:poster');
  });

  it('matches imports by fingerprint unless their full content hashes differ', () => {
    const PM = mediaRegistry();
    const a = 'a'.repeat(64), b = 'b'.repeat(64);
    const project = { assets: { x: { id: 'x', kind: 'audio', name: 'room tone.wav', fingerprint: 'v2:10:f', contentHash: a } }, layers: [] };
    const live = new Map([['x', {}]]);
    // Same sampled fingerprint, different bytes: a new asset, not the old one.
    expect(PM.MediaImport.match(project, live, { kind: 'audio', name: 'room tone 2.wav', fingerprint: 'v2:10:f', contentHash: b }).canonicalId).toBeNull();
    expect(PM.MediaImport.match(project, live, { kind: 'audio', name: 'room tone.wav', fingerprint: 'v2:10:f', contentHash: a }).canonicalId).toBe('x');
    // Without a hash on either side, the fingerprint still decides (legacy engine).
    expect(PM.MediaImport.match(project, live, { kind: 'audio', name: 'room tone.wav', fingerprint: 'v2:10:f' }).canonicalId).toBe('x');
    // A converted copy's hash is of the conversion: re-importing the original upgrades it.
    const converted = { assets: { y: { id: 'y', kind: 'video', name: 'a.mov', fingerprint: 'v2:20:g', contentHash: a, playbackProxy: true } }, layers: [] };
    expect(PM.MediaImport.match(converted, new Map(), { kind: 'video', name: 'a.mov', fingerprint: 'v2:20:g', contentHash: b }).canonicalId).toBe('y');
    expect(PM.MediaImport.originalContentHash({ kind: 'image', format: 'heic', contentHash: a })).toBeNull();
    expect(PM.MediaImport.originalContentHash({ kind: 'image', format: 'heic', rendition: true, contentHash: a })).toBe(a);
    expect(PM.MediaImport.originalContentHash({ kind: 'image', format: 'svg', contentHash: a })).toBe(a);
  });

  it('preserves source time for video trim timing', () => {
    const PM = mediaRegistry();
    const video = { type: 'video', from: 2, d: { trim: 1, speed: 2 } };

    expect(PM.MediaTiming.trimAtStart(video, 5)).toBe(7);
    expect(PM.MediaTiming.earliestStart(video)).toBe(1.5);
  });

  it('trims media before zero while respecting the beginning of its source', () => {
    const PM = mediaRegistry();
    const video = { type: 'video', from: -3, d: { trim: 1, speed: 2 } };
    expect(PM.MediaTiming.earliestStart(video)).toBe(-3.5);
    expect(PM.MediaTiming.trimAtStart(video, -3.5)).toBe(0);
    expect(PM.MediaTiming.trimAtStart(video, -2)).toBe(3);
  });

  it('preserves order without exceeding bounded work concurrency', async () => {
    const PM = mediaRegistry();
    let active = 0;
    let peak = 0;
    const output = await PM.MediaImport.mapBounded([1, 2, 3, 4, 5], 2, async (value: number) => {
      active++;
      peak = Math.max(peak, active);
      await Promise.resolve();
      active--;
      return value * 10;
    });

    expect(output).toEqual([10, 20, 30, 40, 50]);
    expect(peak).toBe(2);
  });

  it('tracks, relinks, and removes model assets referenced by structured extension layers', () => {
    const PM = mediaRegistry();
    const project: any = {
      assets: { old: { id: 'old', kind: 'model' }, current: { id: 'current', kind: 'model' } },
      comps: {},
      layers: [{ id: 'mesh', type: 'extension', d: { data: { assetId: 'old' } } }]
    };
    expect(PM.MediaImport.referenceCount(project, 'old')).toBe(1);
    expect(PM.MediaImport.coalesce(project, 'current', ['old'])).toBe(1);
    expect(project.layers[0].d.data.assetId).toBe('current');
    expect(project.assets.old).toBeUndefined();
    expect(PM.MediaImport.removeAsset(project, 'current')).toEqual({ removedLayers: 1, removedLayerIds: ['mesh'] });
  });

  describe('IndexedDB open', () => {
    /** An IndexedDB whose open answer the test delivers; writes commit at once. */
    function fakeIndexedDB() {
      const opened: any[] = [];
      const db = {
        closed: 0,
        objectStoreNames: { contains: () => true },
        close() { this.closed++; },
        transaction() {
          const tx: any = {
            objectStore: () => ({
              put: () => {
                const req: any = { result: 'stored' };
                queueMicrotask(() => { req.onsuccess?.(); tx.oncomplete?.(); });
                return req;
              },
            }),
          };
          return tx;
        },
      };
      const indexedDB = { open: () => { const req: any = { result: null }; opened.push(req); return req; } };
      const answer = (req: any) => { req.result = db; req.onsuccess(); };
      return { indexedDB, db, opened, answer };
    }
    let clock = 0;
    afterEach(() => {
      vi.useRealTimers();
      vi.restoreAllMocks();
      delete (window as any).indexedDB;
    });
    function install() {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
      clock = 0;
      vi.spyOn(performance, 'now').mockImplementation(() => clock);
      const fake = fakeIndexedDB();
      (window as any).indexedDB = fake.indexedDB;
      return { PM: mediaRegistry(), ...fake };
    }

    it('does not time out an open whose answer waited behind a long main-thread task', async () => {
      const { PM, opened, answer } = install();
      // The first access (an extension import) starts the open...
      const importing = PM.MediaStore.put('import', new Blob(['clip']));
      expect(opened).toHaveLength(1);
      // ...then an 80 s task blocks the page. Its timers come due during the
      // task and run before the queued open answer.
      clock += 80_000;
      vi.advanceTimersByTime(80_000);
      // A project open's media restore now waits on the same connection.
      const restoring = PM.MediaStore.put('restore', new Blob(['media']));
      answer(opened[0]);
      expect(await importing).toBe(true);
      expect(await restoring).toBe(true);
      expect(opened).toHaveLength(1);
    });

    it('still gives up after five responsive seconds and closes a late connection', async () => {
      const { PM, db, opened, answer } = install();
      const put = PM.MediaStore.put('key', new Blob(['x']));
      for (let t = 0; t < 5000; t += 250) { clock += 250; vi.advanceTimersByTime(250); }
      expect(await put).toBe(false);
      answer(opened[0]);
      expect(db.closed).toBe(1);
      // The next access opens again rather than reusing the abandoned attempt.
      const retry = PM.MediaStore.put('key', new Blob(['x']));
      expect(opened).toHaveLength(2);
      answer(opened[1]);
      expect(await retry).toBe(true);
    });
  });
});

