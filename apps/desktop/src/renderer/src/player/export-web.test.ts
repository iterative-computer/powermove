// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createEngine } from './player';
import { buildWebExport, inspectWebExport } from './export-web';
import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import {createObject} from '../core/scene3d/schema';
import {materialPreset} from '../core/scene3d/materials';
import { zipFiles } from './archive';
import { exportActionLabel, exportFieldSupport, normalizeExportDefaults, planExport } from '../core/export-defaults';

function fixture() {
  const PM = createEngine(null);
  PM.proj = PM.mkProject({ name: '</script><script>bad()</script>', w: 100, h: 100, dur: 2 });
  PM.proj.notes = 'private notes'; PM.proj.edits = [{ secret: true }];
  const layer = PM.mkLayer('solid', { name: 'Box' }, PM.proj);
  layer.p['position.x'].kf = [PM.KF(0, 10), PM.KF(1, 90)];
  PM.proj.layers.push(layer);
  PM.MediaStore = { get: async () => null };
  return PM;
}

describe('portable web export', () => {
  it('exports owned scene source, a standalone runtime, and safe preview markup', async () => {
    const PM = fixture(), before = PM.serialize();
    const result = await buildWebExport(PM);
    const layer = result.scene.project.layers[0]!;
    expect(layer.type !== 'audio' && layer.type !== 'group' && layer.p['position.x'].kf.length).toBe(2);
    expect(result.scene.project).not.toHaveProperty('notes');
    expect(result.scene.project).not.toHaveProperty('edits');
    expect(PM.serialize()).toBe(before);
    expect(new TextDecoder().decode(result.files.get('index.html'))).not.toContain('<script>bad()');
    expect(result.files.get('player.js')!.length).toBeGreaterThan(10000);
    expect(result.files.has('PowermoveAnimation.jsx')).toBe(true);
    const handoff = JSON.parse(new TextDecoder().decode(result.files.get('handoff.json')));
    expect(handoff.composition.durationSeconds).toBe(2);
    expect(handoff.referenceTimesSeconds).toEqual([0, 1, 2 - 1 / PM.proj.fps]);
    for (const file of Object.values<string>(handoff.files)) expect(result.files.has(file)).toBe(true);
    expect(result.files.has(handoff.entrypoint)).toBe(true);
    expect(result.files.has('HANDOFF_PROMPT.txt')).toBe(true);
    expect(new DataView(result.bytes.buffer).getUint32(0, true)).toBe(0x04034b50);
  });

  it('fails on missing definitions and media instead of flattening or omitting them', async () => {
    const PM = fixture();
    PM.proj.layers[0].fx.push({ type: 'missing', p: {}, on: true });
    expect(inspectWebExport(PM).errors[0]).toContain('missing');
    await expect(buildWebExport(PM)).rejects.toThrow('missing');
    PM.proj.layers[0].fx = [];
    PM.proj.layers.push(PM.mkLayer('image', { d: { asset: 'missing' } }, PM.proj));
    await expect(buildWebExport(PM)).rejects.toThrow('Missing media');
  });

  it('preserves missing effect placeholders without blocking the editor-equivalent export', async () => {
    const PM = fixture();
    const effect = { id: 'swap', type: 'word-slide-swap', on: true, missing: true,
      p: { progress: PM.P(0, { kf: [PM.KF(0, 0), PM.KF(.6, 100)] }) } };
    PM.proj.layers[0].fx.push(effect);
    const before = PM.serialize();
    const result = await buildWebExport(PM);
    expect(result.scene.effects).toEqual([]);
    expect((result.scene.project.layers[0] as any).fx[0]).toEqual(effect);
    expect(result.scene.warnings.join('\n')).toContain('word-slide-swap');
    expect(PM.serialize()).toBe(before);
  });

  it('captures only used media and removes workstation-specific metadata', async () => {
    const PM = fixture();
    PM.proj.assets = { image: { id: 'image', kind: 'image', name: 'image.svg', format: 'svg', path: '/private/a', storageKey: 'local-only' }, unused: { id: 'unused' } };
    PM.proj.layers.push(PM.mkLayer('image', { d: { asset: 'image' } }, PM.proj));
    PM.MediaStore.get = async () => new Blob(['<svg/>'], { type: 'image/svg+xml' });
    const result = await buildWebExport(PM);
    expect(Object.keys(result.scene.assets)).toEqual(['image']);
    expect(result.scene.project.assets.image).not.toHaveProperty('path');
    expect(result.scene.project.assets.image).not.toHaveProperty('storageKey');
    expect(result.scene.project.assets.image).toMatchObject({ format: 'svg' });
    expect(result.scene.assets.image).toMatch(/\.svg$/);
  });

  it('takes the browser rendition of media whose original the new engine keeps', async () => {
    const PM = fixture();
    PM.proj.assets = { clip: { id: 'clip', kind: 'video', name: 'A001.mov', storageKey: 'media:v2:p', contentHash: 'c'.repeat(64), rendition: true, playbackProxy: false } };
    PM.proj.layers.push(PM.mkLayer('video', { d: { asset: 'clip' } }, PM.proj));
    const originals: string[] = [];
    PM.MediaStore.get = async (meta: any) => { originals.push(meta.id); return new Blob(['prores'], { type: 'video/quicktime' }); };
    (PM.MediaStore as any).rendition = { get: async (hash: string) => (hash === 'c'.repeat(64) ? new Blob(['vp9'], { type: 'video/webm' }) : null) };
    const result = await buildWebExport(PM);
    expect(originals).toEqual([]);
    expect(result.scene.assets.clip).toMatch(/\.webm$/);
    expect(new TextDecoder().decode(result.files.get(result.scene.assets.clip!))).toBe('vp9');
    // No rendition and nothing on screen: say so instead of shipping an unplayable original.
    (PM.MediaStore as any).rendition = { get: async () => null };
    await expect(buildWebExport(PM)).rejects.toThrow('browser-playable');
  });

  describe('browser renditions without a kept copy', () => {
    const HASH = 'b'.repeat(64), OTHER = 'd'.repeat(64);
    const converted = new TextEncoder().encode('playable');
    /* blob: URLs as Chromium treats them: fetch resolves the URL when it is
       called, and a revoked URL no longer resolves. */
    function blobURLs() {
      const urls = new Map<string, Blob>();
      const fetched: string[] = [];
      vi.stubGlobal('fetch', vi.fn((url: string) => {
        fetched.push(url);
        const blob = urls.get(url);
        return blob ? Promise.resolve({ blob: async () => blob }) : Promise.reject(new TypeError('Failed to fetch'));
      }));
      return { urls, fetched };
    }
    function setup(meta: Record<string, unknown>, { enabled = true } = {}) {
      const PM = fixture();
      PM.proj.assets = { clip: { id: 'clip', kind: 'video', name: 'A001.mov', storageKey: 'media:v2:clip', fingerprint: 'fp', rendition: true, playbackProxy: false, ...meta } };
      PM.proj.layers.push(PM.mkLayer('video', { d: { asset: 'clip' } }, PM.proj));
      const live = new Map<string, any>();
      (PM as any).assets = { get: (id: string) => live.get(id) };
      const release = vi.fn(async () => {});
      installBridgeForTests({ media: { readPlaybackProxy: vi.fn(async () => converted), releasePlaybackProxy: release } } as any);
      const store = {
        enabled, keepsOriginals: enabled,
        get: vi.fn(async (): Promise<Blob | null> => new Blob(['prores'], { type: 'video/quicktime' })),
        convert: vi.fn(async (..._args: any[]): Promise<any> => ({ ok: true, token: 'conversion', size: converted.length, type: 'video/webm' })),
        rendition: { get: vi.fn(async (): Promise<Blob | null> => null), put: vi.fn(async () => true) },
      };
      PM.MediaStore = store;
      return { PM, live, store, release };
    }
    const text = async (exported: Awaited<ReturnType<typeof buildWebExport>>) => new TextDecoder().decode(exported.files.get(exported.scene.assets.clip!));
    afterEach(() => { vi.unstubAllGlobals(); resetBridgeForTests(); });

    it('with the engine off, ships the rendition the live asset shows without converting, reading or writing', async () => {
      const { urls } = blobURLs();
      // A restored asset carries no hash: the stored bytes it shows identify it.
      const { PM, live, store } = setup({ contentHash: HASH }, { enabled: false });
      urls.set('blob:shown', new Blob(['shown-vp9'], { type: 'video/webm' }));
      live.set('clip', { id: 'clip', kind: 'video', url: 'blob:shown', storageKey: 'media:v2:clip', fingerprint: 'fp' });
      const exported = await buildWebExport(PM);
      expect(await text(exported)).toBe('shown-vp9');
      expect(exported.scene.assets.clip).toMatch(/\.webm$/);
      expect(store.convert).not.toHaveBeenCalled();
      expect(store.get).not.toHaveBeenCalled();
      expect(store.rendition.put).not.toHaveBeenCalled();
    });

    it('keeps the bytes on screen when the asset is relinked while the export awaits', async () => {
      const { urls, fetched } = blobURLs();
      const { PM, live, store } = setup({ contentHash: HASH });
      urls.set('blob:before', new Blob(['before-relink'], { type: 'video/webm' }));
      live.set('clip', { id: 'clip', kind: 'video', url: 'blob:before', contentHash: HASH });
      store.rendition.get.mockImplementation(async () => {
        // A relink lands during the export's first await: new bytes, old URL revoked.
        urls.delete('blob:before');
        urls.set('blob:after', new Blob(['after-relink'], { type: 'video/webm' }));
        live.set('clip', { id: 'clip', kind: 'video', url: 'blob:after', contentHash: OTHER });
        return null;
      });
      const exported = await buildWebExport(PM);
      expect(fetched).toEqual(['blob:before']);
      expect(await text(exported)).toBe('before-relink');
      expect(store.convert).not.toHaveBeenCalled();
    });

    it('converts by reference, never reading a multi-gigabyte original, when the live asset shows other media', async () => {
      const { urls, fetched } = blobURLs();
      const { PM, live, store, release } = setup({ name: 'A001.mov', contentHash: HASH });
      // The live asset already belongs to other content (a relink before the export began).
      urls.set('blob:other', new Blob(['other-media'], { type: 'video/webm' }));
      live.set('clip', { id: 'clip', kind: 'video', url: 'blob:other', contentHash: OTHER });
      const exported = await buildWebExport(PM);
      expect(fetched).toEqual([]);
      expect(store.convert).toHaveBeenCalledTimes(1);
      // The extension names the decoder; the hash, not a stale link, names the object.
      expect(store.convert).toHaveBeenCalledWith('video', null, { hash: HASH, link: null, name: 'original.mov' });
      expect(store.get).not.toHaveBeenCalled();
      expect(await text(exported)).toBe('playable');
      expect(store.rendition.put).toHaveBeenCalledWith(HASH, expect.any(Blob));
      expect(release).toHaveBeenCalledWith('conversion');
    });

    it('requires full source identity for a hashless native rendition', async () => {
      const { urls, fetched } = blobURLs();
      const { PM, live, store } = setup({ contentHash: HASH });
      urls.set('blob:collision', new Blob(['wrong-original'], { type: 'video/webm' }));
      live.set('clip', { id: 'clip', kind: 'video', url: 'blob:collision', storageKey: 'media:v2:clip', fingerprint: 'fp' });
      expect(await text(await buildWebExport(PM))).toBe('playable');
      expect(fetched).toEqual([]);
      expect(store.convert).toHaveBeenCalledWith('video', null, { hash: HASH, link: null, name: 'original.mov' });
      expect(store.get).not.toHaveBeenCalled();
    });

    it('names the original by its format and passes the link only for linked media', async () => {
      blobURLs();
      const link = { path: '/Volumes/A/A001.mov', hash: HASH, size: 6 };
      const { PM, store } = setup({ name: 'A001.mov', contentHash: HASH, link });
      await buildWebExport(PM);
      expect(store.convert).toHaveBeenLastCalledWith('video', null, { hash: HASH, link: null, name: 'original.mov' });
      PM.proj.assets.clip.storage = 'linked';
      PM.proj.assets.clip.format = 'mov';
      await buildWebExport(PM);
      expect(store.convert).toHaveBeenLastCalledWith('video', null, { hash: HASH, link, name: 'original.mov' });
      expect(store.get).not.toHaveBeenCalled();
    });

    it('converts a kept TIFF still by reference into a PNG', async () => {
      blobURLs();
      const { PM, store } = setup({ kind: 'image', name: 'Scan.tif', format: 'tif', contentHash: HASH });
      PM.proj.layers.at(-1).type = 'image';
      store.convert.mockResolvedValueOnce({ ok: true, token: 'conversion', size: converted.length, type: 'image/png' });
      const exported = await buildWebExport(PM);
      expect(store.convert).toHaveBeenCalledWith('still', null, { hash: HASH, link: null, name: 'original.tif' });
      expect(exported.scene.assets.clip).toMatch(/\.png$/);
      expect(store.get).not.toHaveBeenCalled();
    });

    it('reads and uploads the original only when main has neither the object nor the file', async () => {
      blobURLs();
      const original = new Blob(['prores'], { type: 'video/quicktime' });
      const { PM, store } = setup({ name: 'A001.mov', contentHash: HASH });
      store.get.mockResolvedValue(original);
      store.convert.mockResolvedValueOnce({ ok: false, error: 'The original file is no longer available', missing: true });
      const exported = await buildWebExport(PM);
      expect(store.convert.mock.calls).toEqual([
        ['video', null, { hash: HASH, link: null, name: 'original.mov' }],
        ['video', original, { hash: HASH, link: null, name: 'original.mov' }],
      ]);
      expect(await text(exported)).toBe('playable');
      expect(store.rendition.put).toHaveBeenCalledWith(HASH, expect.any(Blob));
      store.get.mockResolvedValue(null);
      store.convert.mockResolvedValueOnce({ ok: false, error: 'The original file is no longer available', missing: true });
      await expect(buildWebExport(PM)).rejects.toThrow('original media for “A001.mov” is missing');
    });

    it('keeps the shipped engine-off error when no browser copy exists', async () => {
      blobURLs();
      const { PM, store } = setup({ name: 'A001.mov', contentHash: HASH }, { enabled: false });
      await expect(buildWebExport(PM)).rejects.toThrow('Open it in the viewer');
      expect(store.convert).not.toHaveBeenCalled();
      expect(store.get).not.toHaveBeenCalled();
      expect(store.rendition.put).not.toHaveBeenCalled();
    });

    it('reports a conversion failure instead of shipping the original', async () => {
      blobURLs();
      const { PM, store } = setup({ name: 'A001.mov', contentHash: HASH });
      store.convert.mockResolvedValueOnce({ ok: false, error: 'Could not optimize this video for playback' });
      await expect(buildWebExport(PM)).rejects.toThrow('Could not optimize');
      expect(store.get).not.toHaveBeenCalled();
    });
  });

  it('keeps native 3D geometry while excluding packed Blender sources and explaining rendering limits',async()=>{
    const PM=fixture();
    PM.Kernel.layerTypes.register('test',{id:'powermove.3d.object',label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d',role:'object'}});
    const object=createObject('model',{primitive:'box'});object.material.shader=materialPreset('surface');object.blender={assetId:'source',object:'Cube'};
    PM.proj.layers.push(PM.mkLayer('extension',{d:{definition:'powermove.3d.object',data:{object}}},PM.proj));
    PM.proj.assets={source:{id:'source',kind:'model',format:'blend',name:'Model.blend'}};
    PM.proj.render3d={enabled:true,engine:'eevee'};
    const result=await buildWebExport(PM);
    expect(result.scene.assets).toEqual({});
    expect(result.scene.project.assets).toEqual({});
    expect(result.scene.warnings.join('\n')).toContain('native 3D shading');
    expect([...result.files.keys()].some(name=>name.endsWith('.blend'))).toBe(false);
    expect((result.scene.project.layers[1] as any).d.data.object.source).toMatchObject({primitive:'box'});
  });

  it('rejects unsafe zip entry paths', () => {
    expect(() => zipFiles(new Map([['../escape', new Uint8Array()]]))).toThrow('Invalid export path');
  });

  it('uses full-composition source delivery without video settings', () => {
    const opts = normalizeExportDefaults({ format: 'web' }, 30);
    expect(opts.format).toBe('web');
    expect(Object.values(exportFieldSupport('web')).every(v => !v)).toBe(true);
    expect(exportActionLabel('web')).toBe('Export code');
    expect(planExport(opts, { w: 100, h: 100, dur: 2, work: [0, 1] }).note).toContain('full composition');
  });
});


it('packages media beyond 256 MB and preserves ZIP offsets and final media bytes', async () => {
  const PM = fixture();
  PM.proj.assets = { video: { id: 'video', kind: 'video', name: 'large.webm' } };
  PM.proj.layers.push(PM.mkLayer('video', { d: { asset: 'video' } }, PM.proj));
  const data = new Uint8Array(256 * 1024 * 1024 + 1);
  data[data.length - 1] = 123;
  PM.MediaStore.get = async () => ({ size: data.length, arrayBuffer: async () => data.buffer });
  const result = await buildWebExport(PM);
  const view = new DataView(result.bytes.buffer);
  const end = result.bytes.length - 22;
  expect(view.getUint32(end, true)).toBe(0x06054b50);
  const directory = view.getUint32(end + 16, true);
  expect(view.getUint32(directory, true)).toBe(0x02014b50);
  expect(view.getUint32(directory + 24, true)).toBe(data.length);
  const start = 30 + view.getUint16(26, true);
  expect(result.bytes[start + data.length - 1]).toBe(123);
}, 30_000);


it('rejects ZIP fields that would otherwise overflow their file-format widths', () => {
  expect(() => zipFiles(new Map([['a'.repeat(65536), new Uint8Array()]]))).toThrow('path is too long');
  expect(() => zipFiles(new Map(Array.from({ length: 65535 }, (_, i) => [`${i}.txt`, new Uint8Array()])))).toThrow('ZIP64');
  expect(() => zipFiles(new Map([['large.bin', { length: 0xffffffff } as Uint8Array]]))).toThrow('ZIP64');
});
