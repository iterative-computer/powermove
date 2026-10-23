import { EFFECTS } from '../../../extensions/effects-basic/effects';
// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { createEngine } from './player';
import { buildWebExport, inspectWebExport } from './export-web';
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
    const { default: playerSource } = await import('virtual:powermove-player');
    expect(Buffer.from(result.files.get('player.js')!).equals(Buffer.from(playerSource))).toBe(true);
    expect(result.files.has('PowermoveAnimation.jsx')).toBe(true);
    const handoff = JSON.parse(new TextDecoder().decode(result.files.get('handoff.json')));
    expect(handoff.composition.durationSeconds).toBe(2);
    expect(handoff.referenceTimesSeconds).toEqual([0, 1, 2 - 1 / PM.proj.fps]);
    for (const file of Object.values<string>(handoff.files)) expect(result.files.has(file)).toBe(true);
    expect(result.files.has(handoff.entrypoint)).toBe(true);
    expect(result.files.has('HANDOFF_PROMPT.txt')).toBe(true);
    expect(new DataView(result.bytes.buffer).getUint32(0, true)).toBe(0x04034b50);
  });

  it.each(['string', 'object'] as const)('keeps the %s snapshot fixed while the runtime chunk loads', async serialize => {
    const PM = fixture();
    if (serialize === 'object') PM.serialize = () => ({ proj: PM.proj });
    const initialName = PM.proj.name;
    const exporting = buildWebExport(PM);
    PM.proj.name = 'Edited during export';
    PM.proj.layers[0].p['position.x'].kf[0].v = 999;
    const result = await exporting;
    expect(result.scene.project.name).toBe(initialName);
    expect((result.scene.project.layers[0] as any).p['position.x'].kf[0].v).toBe(10);
    expect(JSON.parse(new TextDecoder().decode(result.files.get('scene.json'))).project).toEqual(result.scene.project);
    expect(PM.proj.name).toBe('Edited during export');
    expect(PM.proj.layers[0].p['position.x'].kf[0].v).toBe(999);
  });

  it('preserves nested privacy, ownership, and package bytes for string and object serializers across media waits', async () => {
    const PM = fixture(), nested = PM.mkProject({ name: 'Nested', w: 100, h: 100, dur: 2 });
    const child = PM.mkLayer('solid', { name: 'Nested shape' }, nested);
    nested.layers = [child, PM.mkLayer('image', { d: { asset: 'used' } }, nested)];
    nested.notes = 'private nested notes'; nested.edits = [{ secret: true }];
    nested.assets = {
      used: { id: 'used', kind: 'image', name: 'original.png', path: '/private/source.png', storageKey: 'private-storage' },
      unused: { id: 'unused', name: 'unused.png', path: '/private/unused.png' },
    };
    child.locked_intent = 'private nested intent'; PM.proj.layers[0].locked_intent = 'private root intent';
    PM.proj.layers[0].fx = [{ id: 'snapshot-effect', type: 'snapshot-effect', on: true, p: {} }];
    PM.proj.comps = { nested };
    PM.proj.layers.push(PM.mkLayer('precomp', { d: { comp: 'nested' } }, PM.proj));
    const original = JSON.stringify({ proj: PM.proj, ws: { private: true }, history: { private: true } });
    let reference: Awaited<ReturnType<typeof buildWebExport>> | undefined;
    for (const serialize of ['string', 'object']) {
      const source = JSON.parse(original);
      const definition = { id: 'snapshot-effect', label: 'Original effect', params: [{ k: 'gain', def: 1 }], frag: 'original shader' };
      PM.proj = source.proj;
      PM.Kernel = { effects: new Map([[definition.id, definition]]) };
      PM.serialize = () => serialize === 'string' ? JSON.stringify(source) : source;
      let release!: (blob: Blob) => void;
      const get = vi.fn((_meta: any) => new Promise<Blob>(resolve => { release = resolve; }));
      PM.MediaStore.get = get;
      const exporting = buildWebExport(PM);
      expect(get).toHaveBeenCalledOnce();
      expect(source.proj.layers[0].locked_intent).toBe('private root intent');
      expect(source.proj.comps.nested.layers[0].locked_intent).toBe('private nested intent');
      source.proj.name = 'Edited root'; source.proj.layers[0].p['position.x'].kf[0].v = 999;
      source.proj.comps.nested.name = 'Edited nested'; source.proj.comps.nested.layers[0].d.color = '#123456';
      source.proj.comps.nested.assets.used.name = 'edited.jpg'; definition.params[0]!.def = 999; definition.frag = 'edited shader';
      release(new Blob([new Uint8Array([4, 5, 6])], { type: 'image/png' }));
      const result = await exporting, project = result.scene.project, sub = project.comps.nested!;
      expect(project.name).toBe(JSON.parse(original).proj.name);
      expect((project.layers[0] as any).p['position.x'].kf[0].v).toBe(10);
      expect(sub.name).toBe('Nested'); expect((sub.layers[0] as any).d.color).not.toBe('#123456');
      for (const comp of [project, sub]) {
        expect(comp).not.toHaveProperty('notes'); expect(comp).not.toHaveProperty('edits');
        for (const layer of comp.layers) expect(layer).not.toHaveProperty('locked_intent');
      }
      expect(result.scene).not.toHaveProperty('ws'); expect(result.scene).not.toHaveProperty('history');
      expect(sub.assets).toEqual({});
      expect(project.assets).toEqual({ used: { id: 'used', kind: 'image', name: 'original.png' } });
      expect(get.mock.calls[0]![0]).toMatchObject({ name: 'original.png', storageKey: 'private-storage' });
      expect(result.scene.effects).toEqual([{ ...definition, params: [{ k: 'gain', def: 1 }], frag: 'original shader' }]);
      expect(JSON.parse(new TextDecoder().decode(result.files.get('scene.json')))).toEqual(result.scene);
      if (reference) {
        // Native byte equality checks the complete multi-megabyte runtime/ZIP
        // without spending the test timeout walking typed-array object keys.
        expect([...result.files.keys()]).toEqual([...reference.files.keys()]);
        for (const [name, bytes] of result.files)
          expect(Buffer.from(bytes).equals(Buffer.from(reference.files.get(name)!)), name).toBe(true);
        expect(Buffer.from(result.bytes).equals(Buffer.from(reference.bytes))).toBe(true);
      } else reference = result;
      const beforeReturnMutation = JSON.stringify(source);
      (project.layers[0] as any).p['position.x'].kf[0].v = -123;
      (sub.layers[0] as any).d.color = '#ffffff'; (project.assets.used as any).name = 'returned edit.png';
      result.scene.effects[0]!.params[0].def = -123;
      expect(JSON.stringify(source)).toBe(beforeReturnMutation);
      expect(definition.params[0]!.def).toBe(999);
    }
  });

  it('keeps the selected working precision in portable web scenes', async () => {
    const PM = fixture();
    PM.proj.colorDepth = 32;
    const result = await buildWebExport(PM);
    expect(result.scene.project.colorDepth).toBe(32);
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

  it('keeps root-first and depth-first metadata precedence for nested media and duplicate IDs', async () => {
    const PM = fixture();
    const first = PM.mkProject({ name: 'First', w: 100, h: 100, dur: 2 });
    const deep = PM.mkProject({ name: 'Deep', w: 100, h: 100, dur: 2 });
    const second = PM.mkProject({ name: 'Second', w: 100, h: 100, dur: 2 });
    const asset = (id: string, name: string) => ({ id, name, kind: 'image', path: '/private/source.png' });
    PM.proj.assets = { root: asset('root', 'root.png'), shared: asset('shared', 'root-shared.png'), nested: null };
    first.assets = { shared: asset('shared', 'first-shared.png'), nested: asset('nested', 'first-nested.png') };
    deep.assets = { nested: asset('nested', 'deep-nested.png'), deep: asset('deep', 'first-deep.png') };
    second.assets = { nested: asset('nested', 'second-nested.png'), deep: asset('deep', 'second-deep.png') };
    first.comps = { deep };
    PM.proj.comps = { first, second };
    for (const id of ['root', 'shared', 'nested', 'deep']) PM.proj.layers.push(PM.mkLayer('image', { d: { asset: id } }, PM.proj));
    const read = vi.fn(async (meta: any) => new Blob([meta.name], { type: 'image/png' }));
    PM.MediaStore.get = read;
    const before = PM.serialize();
    const result = await buildWebExport(PM);
    const names = ['root.png', 'root-shared.png', 'first-nested.png', 'first-deep.png'];
    expect(read.mock.calls.map(([meta]) => meta.name)).toEqual(names);
    for (const [index, id] of ['root', 'shared', 'nested', 'deep'].entries()) {
      const location = `assets/media-${index}.png`;
      expect(result.scene.assets[id]).toBe(location);
      expect(new TextDecoder().decode(result.files.get(location))).toBe(names[index]);
      expect(result.scene.project.assets[id]).toMatchObject({ id, name: names[index], kind: 'image' });
      expect(result.scene.project.assets[id]).not.toHaveProperty('path');
    }
    expect(result.scene.project.comps.first!.assets).toEqual({});
    expect(result.scene.project.comps.first!.comps.deep!.assets).toEqual({});
    expect(result.scene.project.comps.second!.assets).toEqual({});
    expect(JSON.parse(new TextDecoder().decode(result.files.get('scene.json')))).toEqual(result.scene);
    expect(PM.serialize()).toBe(before);
    PM.proj.layers.push(PM.mkLayer('image', { d: { asset: 'missing' } }, PM.proj));
    await expect(buildWebExport(PM)).rejects.toThrow('Missing media metadata: missing');
  });

  it('overlaps a bounded number of media reads while preserving snapshot bytes and package order', async () => {
    const PM = fixture();
    const releases = new Map<string, (blob: Blob) => void>();
    const blobs = new Map<string, Blob>();
    for (let index = 0; index < 6; index++) {
      const id = `image-${index}`;
      PM.proj.assets[id] = { id, kind: 'image', name: `${id}.png` };
      PM.proj.layers.push(PM.mkLayer('image', { d: { asset: id } }, PM.proj));
      blobs.set(id, new Blob([`original-${index}`], { type: 'image/png' }));
    }
    PM.MediaStore.get = (meta: any) => new Promise<Blob>(resolve => releases.set(meta.id, resolve));
    const exporting = buildWebExport(PM);
    await vi.waitFor(() => expect(releases.size).toBe(4));
    PM.proj.assets['image-0'].name = 'edited-after-snapshot.jpg';
    for (let index = 3; index >= 0; index--) releases.get(`image-${index}`)!(blobs.get(`image-${index}`)!);
    await vi.waitFor(() => expect(releases.size).toBe(6));
    for (let index = 5; index >= 4; index--) releases.get(`image-${index}`)!(blobs.get(`image-${index}`)!);
    const result = await exporting;
    expect([...result.files.keys()].slice(0, 6)).toEqual(Array.from({ length: 6 }, (_, index) => `assets/media-${index}.png`));
    for (let index = 0; index < 6; index++) {
      const path = `assets/media-${index}.png`;
      expect(result.scene.assets[`image-${index}`]).toBe(path);
      expect(new TextDecoder().decode(result.files.get(path))).toBe(`original-${index}`);
    }
    expect(result.scene.project.assets['image-0']).toMatchObject({ name: 'image-0.png' });
  });

  it('drains a failed media batch before rejecting without starting further reads', async () => {
    const PM = fixture();
    const failures = new Map<string, (error: Error) => void>();
    for (let index = 0; index < 5; index++) {
      const id = `image-${index}`;
      PM.proj.assets[id] = { id, kind: 'image', name: `${id}.png` };
      PM.proj.layers.push(PM.mkLayer('image', { d: { asset: id } }, PM.proj));
    }
    PM.MediaStore.get = (meta: any) => new Promise<Blob>((_resolve, reject) => failures.set(meta.id, reject));
    let settled = false;
    const exporting = buildWebExport(PM);
    exporting.then(() => { settled = true; }, () => { settled = true; });
    const rejected = expect(exporting).rejects.toThrow('first read failed');
    await vi.waitFor(() => expect(failures.size).toBe(4));
    failures.get('image-2')!(new Error('later read failed'));
    await Promise.resolve(); await Promise.resolve();
    expect(settled).toBe(false);
    failures.get('image-0')!(new Error('first read failed'));
    for (const index of [1, 3]) failures.get(`image-${index}`)!(new Error('later read failed'));
    await rejected;
    expect(failures.has('image-4')).toBe(false);
  });

  it('loads four font subsets together while preserving family, face, metadata, and file order', async () => {
    const PM = fixture();
    for (const family of ['Fixture B', 'Fixture A']) PM.proj.layers.push(PM.mkLayer('text', { d: { font: family } }, PM.proj));
    const stylesheet = document.createElement('style');
    stylesheet.textContent = Array.from({ length: 8 }, (_, index) => `@font-face { font-family: "Fixture ${index % 2 ? 'B' : 'A'}"; src: url("https://fonts.example.test/subset-${index}.woff2"); font-weight: ${(index + 1) * 100}; font-style: ${index % 2 ? 'italic' : 'normal'}; unicode-range: U+${index}000-${index}FFF; }`).join('\n');
    document.head.append(stylesheet);
    const bodies = new Map<number, (bytes: ArrayBuffer) => void>();
    const fetchFont = vi.fn(async (url: URL) => {
      const index = Number(/subset-(\d+)/.exec(url.href)![1]);
      return { ok: true, arrayBuffer: () => new Promise<ArrayBuffer>(resolve => bodies.set(index, resolve)) };
    });
    vi.stubGlobal('fetch', fetchFont);
    let exporting: ReturnType<typeof buildWebExport> | undefined;
    const release = (index: number) => bodies.get(index)!(new Uint8Array([index, 255 - index]).buffer);
    try {
      let settled = false;
      exporting = buildWebExport(PM);
      exporting.then(() => { settled = true; }, () => { settled = true; });
      await vi.waitFor(() => expect([...bodies.keys()]).toEqual([1, 3, 5, 7]));
      (stylesheet.sheet!.cssRules[1] as CSSFontFaceRule).style.setProperty('font-weight', '999');
      for (const index of [5, 3, 1]) release(index);
      await Promise.resolve(); await Promise.resolve();
      expect(fetchFont).toHaveBeenCalledTimes(4);
      expect(settled).toBe(false);
      release(7);
      await vi.waitFor(() => expect([...bodies.keys()]).toEqual([1, 3, 5, 7, 0, 2, 4, 6]));
      for (const index of [6, 4, 2, 0]) release(index);
      const result = await exporting;
      const order = [1, 3, 5, 7, 0, 2, 4, 6];
      expect(result.scene.fonts).toEqual(order.map((index, file) => ({ family: `Fixture ${index % 2 ? 'B' : 'A'}`, src: `assets/font-${file}.bin`, weight: String((index + 1) * 100), style: index % 2 ? 'italic' : 'normal', unicodeRange: `U+${index}000-${index}FFF` })));
      expect([...result.files.keys()].slice(0, 8)).toEqual(order.map((_, file) => `assets/font-${file}.bin`));
      for (const [file, index] of order.entries()) expect(result.files.get(`assets/font-${file}.bin`)).toEqual(new Uint8Array([index, 255 - index]));
      expect(result.scene.warnings).toEqual([]);
    } finally {
      fetchFont.mockImplementation(async () => ({ ok: false, arrayBuffer: async () => new ArrayBuffer(0) }));
      for (const index of bodies.keys()) release(index);
      await exporting?.catch(() => undefined);
      stylesheet.remove(); vi.unstubAllGlobals();
    }
  });

  it('preserves font fallback warnings for failed requests and body reads in family order', async () => {
    const PM = fixture();
    const families = ['Fixture Partly Available', 'Fixture Failed', 'Fixture Without URL', 'Fixture No Face'];
    for (const family of families) PM.proj.layers.push(PM.mkLayer('text', { d: { font: family } }, PM.proj));
    const stylesheet = document.createElement('style');
    stylesheet.textContent = ['ok', 'http', 'request', 'body'].map(kind => `@font-face { font-family: "Fixture Partly Available"; src: url("https://fonts.example.test/${kind}.woff2"); }`).join('\n')
      + '\n@font-face { font-family: "Fixture Failed"; src: url("https://fonts.example.test/body.woff2"); }'
      + '\n@font-face { font-family: "Fixture Without URL"; src: local("Fixture Without URL"); }';
    document.head.append(stylesheet);
    const bytes = new Uint8Array([3, 1, 4]);
    const fetchFont = vi.fn(async (url: URL) => {
      if (url.pathname.includes('request')) throw new Error('unavailable');
      return { ok: !url.pathname.includes('http'), arrayBuffer: async () => {
        if (url.pathname.includes('body')) throw new Error('body unavailable');
        return bytes.buffer;
      } };
    });
    vi.stubGlobal('fetch', fetchFont);
    try {
      const result = await buildWebExport(PM);
      expect(fetchFont).toHaveBeenCalledTimes(5);
      expect(result.scene.fonts).toEqual([{ family: families[0], src: 'assets/font-0.bin', weight: '400', style: 'normal', unicodeRange: undefined }]);
      expect(result.files.get('assets/font-0.bin')).toEqual(bytes);
      expect(result.scene.warnings).toEqual(families.slice(1).map(family => `Font “${family}” is not bundled. Supply this font in your app to preserve text layout.`));
    } finally { stylesheet.remove(); vi.unstubAllGlobals(); }
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
