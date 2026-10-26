import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from '../__tests__/make-pm';
import { install as installMedia } from '../core/media';
import { install, videoImportFailureMessage, waitForPresentedVideoFrame } from './raster';
import { restoreProjectFileStream } from '../core/project-file';

function rasterRegistry(extra: Partial<PMRegistry> = {}): PMRegistry {
  const context2d: any = {
    font: '', letterSpacing: '', textBaseline: '', textAlign: '', fillStyle: '',
    scale() {}, fillText() {},
    measureText(text: string) {
      const width = Math.max(1, text.length * 48);
      const left = this.textAlign === 'center' ? width / 2 : this.textAlign === 'right' ? width : 0;
      return {
        width,
        actualBoundingBoxLeft: left,
        actualBoundingBoxRight: width - left,
        actualBoundingBoxAscent: 78,
        actualBoundingBoxDescent: 18,
      };
    },
  };
  vi.stubGlobal('window', {
    document: {
      createElement(_tag: string) {
        return { width: 0, height: 0, getContext: () => context2d };
      },
    },
  });
  const PM: PMRegistry = {
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    ...extra,
  };
  install(PM);
  return PM;
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  resetBridgeForTests();
});

describe('legacy raster install', () => {
  it.each(['fingerprint pending', 'persistence queued', 'persistence started'])('preserves a native file media revision when older recovery has %s', async phase => {
    const data = new Uint8Array([1, 2, 3]);
    let record: { blob: Blob; revision: string } | undefined;
    let recoveryStarted!: () => void, releaseRecovery!: () => void;
    const started = new Promise<void>(resolve => { recoveryStarted = resolve; });
    const released = new Promise<void>(resolve => { releaseRecovery = resolve; });
    let storageQueue = Promise.resolve();
    let localWrites = 0;
    const PM = rasterRegistry();
    (window as any).File = File;
    (window as any).navigator = { hardwareConcurrency: 4 };
    installBridgeForTests({ media: {
      openLocalSource: async () => ({ token: 'source', size: data.length }),
      readCloudSource: async () => data,
      releaseCloudSource: async () => undefined,
    } } as any);
    vi.stubGlobal('navigator', { storage: {} });
    const meta = { id: 'clip', name: 'clip.wav', kind: 'audio', storageKey: 'media:same-source', sourcePath: '/local/clip.wav', fingerprint: 'same-source' };
    PM.proj = { id: 'recovering', assets: { clip: meta }, layers: [] };
    PM.MediaImport = {
      mapBounded: async (items: any[], _limit: number, action: (item: any) => Promise<any>) => Promise.all(items.map(action)),
      fingerprint: async () => {
        if (phase === 'fingerprint pending') { recoveryStarted(); await released; }
        return 'same-source';
      },
      storageKeyFor: (fingerprint: string) => `media:${fingerprint}`,
      posterKeyFor: (key: string) => `${key}:poster`,
    };
    PM.MediaStore = {
      get: async () => record?.blob || null,
      put: (_id: string, blob: Blob, _meta: any, revision?: string) => {
        if (!revision) localWrites++;
        // IndexedDB serializes readwrite transactions on the media store.
        const write = storageQueue.then(async () => {
          if (!revision && phase === 'persistence started') { recoveryStarted(); await released; }
          record = { blob, revision: revision || 'local-recovery-revision' };
        });
        storageQueue = write;
        return write.then(() => true);
      },
    };
    PM.Audio = { accepts: () => true, prepareAsset: async ({ id, name, blob }: any) => {
      if (phase === 'persistence queued') recoveryStarted();
      return { id, name, kind: 'audio', audioBlob: blob, size: blob.size };
    }, disposeAsset: vi.fn(), rebalanceCache() {} };
    PM.touch = vi.fn(); PM.bus = { emit: vi.fn() }; PM.invalidate = vi.fn();
    const recovery = PM.assets.restoreProject(PM.proj);
    await started;
    // Native opens stage media before publishing their incoming project. The
    // old project's source restore still sees itself as current during staging.
    PM.projectLoading = { id: 'incoming', generation: 1 };
    PM.assets.cancelRestores();
    const nativeRestore = restoreProjectFileStream({ proj: { assets: { clip: meta } } },
      [{ id: 'clip', type: 'audio/wav', offset: 0, length: data.length, revision: 'native-saved-revision' }],
      PM.MediaStore, async () => data);
    if (phase === 'persistence started') releaseRecovery();
    await nativeRestore;
    expect(record?.revision).toBe('native-saved-revision');
    releaseRecovery();
    await recovery;
    expect(record?.revision).toBe('native-saved-revision');
    expect(PM.assets.map.size).toBe(0);
    expect(localWrites).toBe(phase === 'persistence started' ? 1 : 0);
  });

  it('keeps SVG sources with the same asset id separate across warm projects', () => {
    const PM = rasterRegistry();
    const context = (window as any).document.createElement('canvas').getContext('2d');
    context.clearRect = vi.fn(); context.drawImage = vi.fn();
    const first = { id: 'shared-svg', el: {} }, second = { id: 'shared-svg', el: {} };
    const a = PM.rasterSvgAsset(first, 64, 32), b = PM.rasterSvgAsset(second, 64, 32);
    expect(a.key).not.toBe(b.key);
    expect(PM.rasterSvgAsset(first, 64, 32)).toBe(a);
    expect(PM.rasterSvgAsset(second, 64, 32)).toBe(b);
    expect(context.drawImage).toHaveBeenCalledTimes(2);
  });
  it('releases obsolete GPU assets when a project closes', () => {
    const dropTextures = vi.fn(), dropMesh = vi.fn();
    const PM = rasterRegistry({ GL: { dropTextures, dropMesh } });
    PM.assets.map.set('mesh', { id: 'mesh', kind: 'model' });
    PM.assets.clear();
    expect(dropTextures).toHaveBeenCalledWith('a:');
    expect(dropTextures).toHaveBeenCalledWith('video:');
    expect(dropMesh).toHaveBeenCalledWith('mesh');
  });
  it('unloads disposed source and preview video decoders exactly once', () => {
    const PM = rasterRegistry();
    const source = { pause: vi.fn(), removeAttribute: vi.fn(), load: vi.fn() };
    const preview = { pause: vi.fn(), removeAttribute: vi.fn(), load: vi.fn() };
    const revokeObjectURL = vi.fn();
    (window as any).URL = { revokeObjectURL };
    PM.assets.map.set('video', { id: 'video', kind: 'video', url: 'blob:source', el: source,
      preview: { el: preview, url: 'blob:preview' } });
    PM.assets.clear();
    expect(source.removeAttribute).toHaveBeenCalledWith('src');
    expect(source.load).toHaveBeenCalledTimes(1);
    expect(preview.removeAttribute).toHaveBeenCalledWith('src');
    expect(preview.load).toHaveBeenCalledTimes(1);
    PM.assets.clear();
    expect(source.load).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledTimes(2);
  });
  it('reuses identical capped text bitmaps at larger zoom levels without changing lower-density sources', () => {
    const PM = rasterRegistry();
    const layer = { type: 'text', d: { text: 'W'.repeat(128), font: 'sans-serif', size: 18, color: '#ffffff' } };
    const low = PM.raster(layer, 1), capped = PM.raster(layer, 8);
    expect(capped.cv.width).toBe(8192);
    expect(PM.raster(layer, 16)).toBe(capped);
    expect(PM.raster(layer, 32)).toBe(capped);
    expect(low.key).not.toBe(capped.key);
    expect(PM.raster(layer, 1)).toBe(low);
    expect(low.selection).toEqual(capped.selection);
  });

  it('measures plain text without allocating its zoomed bitmap and invalidates changed typography', () => {
    const PM = rasterRegistry();
    const layer = { type: 'text', d: { text: 'Zoomed typography', font: 'sans-serif', size: 18, color: '#ffffff' } };
    const first = PM.textRasterGeometry(layer, 16);
    expect(PM.rasterStats().bytes).toBe(0);
    const full = PM.raster(layer, 16);
    expect(first).toMatchObject({ w: full.w, h: full.h, anchorX: full.anchorX, anchorY: full.anchorY, selection: full.selection });
    expect(first.width).toBe(full.cv.width);
    expect(first.height).toBe(full.cv.height);
    expect(PM.textRasterGeometry(layer, 2).selection).toBe(first.selection);
    layer.d.text += ' longer';
    expect(PM.textRasterGeometry(layer, 16).w).toBeGreaterThan(first.w);
    layer.d.text = 'Zoomed typography';
    PM.rasterClear();
    expect(PM.textRasterGeometry(layer, 16).selection).not.toBe(first.selection);
    expect(PM.textRasterGeometry(layer, 16).selection).toEqual(first.selection);
  });

  it('keeps hundreds of small sources warm without exceeding the byte budget', () => {
    const PM = rasterRegistry();
    const layers = Array.from({ length: 670 }, (_, index) => ({
      type: 'text', d: { text: `Icon ${index}`, font: 'sans-serif', size: 8, color: '#ffffff' },
    }));
    const first = layers.map(layer => PM.raster(layer, .25));
    expect(PM.rasterStats().size).toBe(670);
    expect(PM.rasterStats().bytes).toBeLessThan(PM.rasterStats().maxBytes);
    layers.forEach((layer, index) => expect(PM.raster(layer, .25)).toBe(first[index]));
  });

  it('releases CPU canvases under pressure while allowing uploaded pixels to be reused', () => {
    let provider: any;
    const dropTextures = vi.fn();
    const PM = rasterRegistry({
      GL: { dropTextures },
      Memory: { budget: () => 1024 * 1024, register: (_name: string, value: any) => { provider = value; } },
    });
    const layer = (text: string) => ({ type: 'text', d: { text, font: 'sans-serif', size: 8, color: '#ffffff' } });
    const first = layer('First'), second = layer('Second');
    const raster = PM.raster(first);
    const { cv, ...uploaded } = raster;
    PM.raster(second);
    provider.trim(0);
    expect(cv.width).toBe(0);
    expect(PM.rasterStats().size).toBe(1);
    expect(dropTextures).not.toHaveBeenCalled();
    const reuse = vi.fn((key: string) => key === uploaded.key ? uploaded : undefined);
    expect(PM.raster(first, 1, 0, reuse)).toBe(uploaded);
    expect(PM.rasterStats().size).toBe(1);
    // Picking/export callers that require an actual bitmap still get one.
    expect(PM.raster(first).cv.width).toBeGreaterThan(0);
    first.d.text = 'Changed';
    expect(PM.raster(first, 1, 0, reuse).key).not.toBe(uploaded.key);
    PM.rasterClear();
    expect(dropTextures).toHaveBeenCalledWith('r:');
    expect(PM.rasterStats()).toMatchObject({ size: 0, bytes: 0 });
  });

  it('remeasures anchored typography instead of reapplying an uploaded anchor offset', () => {
    const PM = rasterRegistry({ ev: () => 0 });
    const layer = { type: 'text', d: {
      text: 'Anchored', font: 'sans-serif', size: 20, color: '#ffffff',
      fontAnchorBounds: { x0: -20, y0: -10, x1: 100, y1: 30 },
    } };
    const before = PM.raster(layer);
    const { cv: _canvas, ...uploaded } = before;
    PM.rasterClear();
    const reuse = vi.fn(() => uploaded);
    const after = PM.raster(layer, 1, 0, reuse);
    expect(reuse).not.toHaveBeenCalled();
    expect(after.cv.width).toBeGreaterThan(0);
    expect(after.selection).toEqual(before.selection);
    expect(after.fontOffset).toEqual(before.fontOffset);
  });

  it('recognizes SVG files even when the native picker omits their MIME type', () => {
    const PM = rasterRegistry();

    expect(PM.assetKind({ name: 'wordmark.svg', type: '' })).toBe('image');
    expect(PM.assetKind({ name: 'wordmark.SVG', type: 'application/octet-stream' })).toBe('image');
    expect(PM.assetKind({ name: 'wordmark', type: 'image/svg+xml' })).toBe('image');
  });

  it('selects actionable codec warnings for mov and mp4 decode failures', () => {
    const mov = videoImportFailureMessage('prores-4444.MOV', {
      code: 4,
      message: ''
    });
    expect(mov).toContain("file's codec is not supported by this build");
    expect(mov).toMatch(/ProRes.*transcod/i);

    const mp4 = videoImportFailureMessage('unsupported.mp4', { code: 3, message: '' });
    expect(mp4).toContain("file's codec is not supported by this build");
    expect(mp4).toMatch(/transcode.*H\.264/i);
    expect(videoImportFailureMessage('unsupported.mp4', new Error('DEMUXER_ERROR_NO_SUPPORTED_STREAMS')))
      .toBe(mp4);
  });

  it('keeps the generic video failure for other formats and non-codec errors', () => {
    expect(videoImportFailureMessage('clip.webm', { code: 3, message: 'decode failed' }))
      .toBe('Could not read this video file');
    expect(videoImportFailureMessage('clip.mov', { code: 2, message: 'network error' }))
      .toBe('Could not read this video file');
  });

  it('distinguishes a real presented video frame from an audio-only advancing MOV', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('window', { setTimeout, clearTimeout });
    let presented: (() => void) | undefined;
    const playable = {
      requestVideoFrameCallback(callback: () => void) { presented = callback; return 7; },
      cancelVideoFrameCallback: vi.fn(),
      play: vi.fn().mockResolvedValue(undefined),
      pause: vi.fn(),
    };
    const success = waitForPresentedVideoFrame(playable, 1000);
    presented?.();
    await expect(success).resolves.toBe(true);
    expect(playable.pause).toHaveBeenCalledOnce();

    const audioOnly = {
      requestVideoFrameCallback: vi.fn(() => 9),
      cancelVideoFrameCallback: vi.fn(),
      play: vi.fn().mockResolvedValue(undefined),
      pause: vi.fn(),
    };
    const failure = waitForPresentedVideoFrame(audioOnly, 1000);
    await vi.advanceTimersByTimeAsync(1000);
    await expect(failure).resolves.toBe(false);
    expect(audioOnly.cancelVideoFrameCallback).toHaveBeenCalledWith(9);
  });

  it('keeps padded raster textures and tight text selection bounds', () => {
    const PM = rasterRegistry();
    const text = { text: 'Powermove', font: 'SF Pro Display', weight: 650, size: 100, tracking: 0, leading: 1, color: '#fff', align: 'center', italic: false };
    const raster = PM.raster({ type: 'text', d: text });

    expect(raster.h).toBeGreaterThan(250);
    expect(raster.selection.h).toBeLessThan(raster.h * .5);
    expect(raster.selection.w).toBeLessThan(raster.w);
    expect(raster.selection.w).toBeGreaterThan(350);
    expect(Math.abs(raster.selection.x0 + raster.selection.x1)).toBeLessThan(1);

    const left = PM.raster({ type: 'text', d: { ...text, align: 'left' } }).selection;
    const right = PM.raster({ type: 'text', d: { ...text, align: 'right' } }).selection;
    expect(left.x0 < 0 && left.x0 > -12 && left.x1 > 350).toBe(true);
    expect(right.x1 > 0 && right.x1 < 12 && right.x0 < -350).toBe(true);

    const layout = PM.textLayout({ ...text, text: 'AB\nC', leading: 1.1 });
    expect(layout.characters.map((piece: any) => piece.text)).toEqual(['A', 'B', 'C']);
  });

  it('wraps paragraph text inside the Type tool box and keeps the box selectable', () => {
    const PM = rasterRegistry();
    const paragraph = PM.raster({
      type: 'text',
      d: {
        text: 'one two three four', font: 'SF Pro Display', weight: 500, size: 40,
        tracking: 0, leading: 1, color: '#fff', align: 'left', italic: false,
        boxWidth: { v: 130, kf: [], expr: null }, boxHeight: { v: 100, kf: [], expr: null },
      },
    });

    expect(paragraph.selection.x0).toBeLessThan(0);
    expect(paragraph.selection.x1).toBeGreaterThan(130);
    expect(paragraph.selection.y1).toBeGreaterThan(100);
    expect(paragraph.w).toBeGreaterThan(130);
    expect(paragraph.h).toBeGreaterThan(100);

    const centered = PM.raster({
      type: 'text',
      d: {
        text: 'centered', font: 'SF Pro Display', weight: 500, size: 40,
        tracking: 0, leading: 1, color: '#fff', align: 'center', italic: false,
        boxWidth: { v: 130, kf: [], expr: null }, boxHeight: { v: 100, kf: [], expr: null },
      },
    }).selection;
    expect(Math.abs(centered.x0 + centered.x1)).toBeLessThan(1);
  });

  it('includes every wrapped line in auto-height paragraph selection bounds', () => {
    const PM = rasterRegistry();
    const layer = {
      type: 'text', d: {
        text: 'one two three four', font: 'SF Pro Display', weight: 500, size: 40,
        tracking: 0, leading: 1, color: '#fff', align: 'left',
        boxWidth: 130, boxHeight: 0,
      },
    };
    const lines = PM.textLayout(layer).lines;
    expect(lines.length).toBeGreaterThan(1);
    const bounds = PM.raster(layer).selection;
    expect(bounds.y0).toBeLessThan(0);
    expect(bounds.y1).toBeGreaterThan(lines.length * 40);
  });

  it('reports caret layout lines with exact source offsets that match the painted wrapping', () => {
    const PM = rasterRegistry();
    const layer = {
      type: 'text', d: {
        text: 'one two  three\n\nfour', font: 'SF Pro Display', weight: 500, size: 40,
        tracking: 0, leading: 1.2, color: '#fff', align: 'left',
        boxWidth: 130, boxHeight: 0,
      },
    };
    const caret = PM.textCaretLayout(layer);
    const painted = PM.textLayout(layer).lines.map((line: any) => line.text);
    expect(caret.lines.filter((line: any) => line.text.length).map((line: any) => line.text)).toEqual(painted);
    // Every line starts where its text sits in the source string, including
    // after the double space dropped at a wrap and across the empty paragraph.
    for (const line of caret.lines) expect(layer.d.text.slice(line.start, line.start + line.text.length)).toBe(line.text);
    expect(caret.lines.some((line: any) => line.text === '' && line.start === 15)).toBe(true);
    const last = caret.lines.at(-1);
    expect(last.start + last.text.length).toBe(layer.d.text.length);
    expect(caret.lines.find((line: any) => line.start === 16)).toBeTruthy();
    expect(caret.lineHeight).toBe(48);
    expect(caret.length).toBe(layer.d.text.length);
    const first = caret.lines[0];
    expect(first.boundaries[0]).toEqual({ index: 0, x: 0 });
    expect(first.boundaries.at(-1)).toMatchObject({ index: first.text.length });
    expect(first.boundaries.at(-1).x).toBeCloseTo(first.width, 5);
    for (let k = 1; k < first.boundaries.length; k++) expect(first.boundaries[k].x).toBeGreaterThan(first.boundaries[k - 1].x);

    const centered = PM.textCaretLayout({ ...layer, d: { ...layer.d, boxWidth: 0, text: 'AB' } });
    expect(centered.lines[0].x).toBe(0);
    const right = PM.textCaretLayout({ ...layer, d: { ...layer.d, boxWidth: 0, text: 'AB', align: 'right' } });
    expect(right.lines[0].x).toBeCloseTo(-right.lines[0].width, 5);
  });

  it('retries a text raster that painted nothing instead of caching the blank bitmap', () => {
    let alpha = 0;
    const context2d: any = {
      font: '', letterSpacing: '', textBaseline: '', textAlign: '', fillStyle: '',
      scale() {}, fillText() {}, drawImage() {},
      getImageData: () => ({ data: new Uint8ClampedArray(24 * 24 * 4).fill(alpha) }),
      measureText: (text: string) => ({ width: Math.max(1, text.length * 48), actualBoundingBoxLeft: 0, actualBoundingBoxRight: text.length * 48, actualBoundingBoxAscent: 78, actualBoundingBoxDescent: 18 }),
    };
    const timers: Array<() => void> = [];
    vi.stubGlobal('window', {
      document: { createElement: () => ({ width: 0, height: 0, getContext: () => context2d }), fonts: { ready: { then(fn: () => void) { timers.push(fn); } } } },
      setTimeout: (fn: () => void) => { timers.push(fn); return 1; },
    });
    const invalidate = vi.fn();
    const PM: PMRegistry = { clamp: (v: number, a: number, b: number) => Math.max(a, Math.min(b, v)), invalidate, time: 0 } as any;
    install(PM);
    const layer = { type: 'text', d: { text: 'Hello', font: 'Loading Face', weight: 500, size: 40, tracking: 0, leading: 1, color: '#fff', align: 'left' } };
    const now = vi.spyOn(Date, 'now').mockReturnValue(1000);
    const first = PM.raster(layer);
    expect(first.blank).toBe(true);
    // The blank bitmap is reused until the retry moment, never as a GPU stub.
    expect(PM.raster(layer, 1, 0, () => ({ ...first, blank: true }))).toBe(first);
    timers.splice(0).forEach(fn => fn());
    expect(invalidate).toHaveBeenCalled();
    alpha = 255;
    now.mockReturnValue(1300);
    const second = PM.raster(layer);
    expect(second).not.toBe(first);
    expect(second.blank).toBe(false);
    expect(PM.raster(layer)).toBe(second);
    now.mockRestore();
  });

  it('does not read GPU pixels for loaded fonts on a healthy canvas', () => {
    const PM = rasterRegistry();
    const context = window.document.createElement('canvas').getContext('2d') as any;
    context.isContextLost = () => false;
    context.drawImage = vi.fn();
    context.getImageData = vi.fn(() => { throw new Error('Unexpected synchronous readback'); });
    const check = vi.fn(() => true);
    (window.document as any).fonts = { check };
    for (const text of ['Healthy font', '世界', 'New text at another density']) {
      const layer = { type: 'text', d: { text, font: 'sans-serif', size: 18, color: '#fff' } };
      for (const scale of [1, 2, 8]) expect(PM.raster(layer, scale).blank).toBe(false);
      expect(check).toHaveBeenCalledWith(context.font, text);
    }
    expect(context.getImageData).not.toHaveBeenCalled();
    expect(context.drawImage).not.toHaveBeenCalled();
  });

  it.each(['loading font', 'lost backing store'])('retains retry and GPU-stub rejection for a %s', failure => {
    vi.useFakeTimers();
    const invalidate = vi.fn(), PM = rasterRegistry({ invalidate });
    const context = window.document.createElement('canvas').getContext('2d') as any;
    let recovered = false;
    context.isContextLost = () => failure === 'lost backing store' && !recovered;
    context.drawImage = vi.fn();
    context.getImageData = vi.fn(() => ({ data: new Uint8ClampedArray(24 * 24 * 4) }));
    (window.document as any).fonts = { check: () => failure !== 'loading font' || recovered };
    (window as any).setTimeout = setTimeout;
    const layer = { type: 'text', d: { text: 'Recover me', font: 'sans-serif', size: 24, color: '#fff' } };
    const first = PM.raster(layer);
    expect(first.blank).toBe(true);
    expect(context.getImageData).toHaveBeenCalledTimes(failure === 'loading font' ? 1 : 0);
    vi.advanceTimersByTime(300);
    expect(invalidate).toHaveBeenCalledWith('render');
    recovered = true;
    const next = PM.raster(layer, 1, 0, () => ({ ...first, blank: true }));
    expect(next).not.toBe(first);
    expect(next.blank).toBe(false);
    expect(PM.raster(layer)).toBe(next);
  });

  it('atomically replaces media in place and restores metadata plus runtime with one Undo and Redo', async () => {
    const disposed: any[] = [];
    const oldRuntime = { id: 'asset-1', name: 'old.wav', kind: 'audio', marker: 'old' };
    const newRuntime = { id: 'asset-1', name: 'new.wav', kind: 'audio', marker: 'new', persistBlob: new Blob(['new']) };
    vi.stubGlobal('window', {
      document: { createElement: () => ({ getContext: () => ({}) }) },
      navigator: { hardwareConcurrency: 4 },
      URL: { createObjectURL: () => 'blob:test', revokeObjectURL: vi.fn() },
      powermove: { media: { sourcePath: () => '/replacement/new.wav' } },
      setTimeout, clearTimeout,
    });
  installBridgeForTests((window as any).powermove);
    const PM = makePM('core/history', 'gl/raster');
    const animatedLayer = {
      id: 'layer-1', type: 'audio', from: 4, dur: 8,
      d: { asset: 'asset-1', trim: 1.5 },
      p: { opacity: { v: 100, kf: [{ i: 'key-1', t: 0, v: 35 }], expr: null } },
    };
    const layerBefore = JSON.stringify(animatedLayer);
    const projectA = PM.proj = {
      id: 'project-1', revision: 2,
      assets: { 'asset-1': { id: 'asset-1', name: 'old.wav', kind: 'audio', storageKey: 'media:old', size: 3, persisted: true } },
      layers: [animatedLayer], comps: {},
    };
    PM.assets.map.set('asset-1', oldRuntime);
    PM.MediaImport = { fingerprint: vi.fn(async () => 'new-fingerprint'), storageKeyFor: (value: string) => `media:${value}` };
    PM.MediaStore = { put: vi.fn(async () => true) };
    PM.Audio = {
      accepts: () => true,
      prepareAsset: vi.fn(async () => newRuntime),
      disposeAsset: vi.fn((asset: any) => disposed.push(asset)),
      pause: vi.fn(), rebalanceCache: vi.fn(),
    };
    PM.touch = vi.fn();
    PM.autosave = vi.fn();
    const viewer = { preview: { clear: vi.fn() } };
    PM.Kernel.services.register('viewer', viewer);
    PM.sel = { layers: [], keys: [], chan: null };
    PM.replaceProject = (project: any) => { PM.proj = project; };

    const file = { name: 'new.wav', type: 'audio/wav', size: 3 };
    const stages: string[] = [];
    await PM.assets.replace('asset-1', file, { onStage: (label: string) => stages.push(label) });

    expect(stages).toEqual(['Reading file', 'Preparing media', 'Saving media']);
    expect(PM.proj.assets['asset-1']).toMatchObject({ id: 'asset-1', name: 'new.wav', kind: 'audio', sourcePath: '/replacement/new.wav', persisted: true });
    expect(PM.assets.get('asset-1')).toBe(newRuntime);

    PM.MediaStore.put.mockResolvedValueOnce(false);
    const failedRuntime = { id: 'asset-1', name: 'failed.wav', kind: 'audio' };
    PM.Audio.prepareAsset.mockResolvedValueOnce(failedRuntime);
    const metadataBeforeFailure = JSON.stringify(PM.proj.assets);
    await expect(PM.assets.replace('asset-1', { name: 'failed.wav', type: 'audio/wav', size: 3 }))
      .rejects.toThrow('Could not store the replacement media');
    expect(JSON.stringify(PM.proj.assets)).toBe(metadataBeforeFailure);
    expect(PM.assets.get('asset-1')).toBe(newRuntime);
    expect(disposed).toContain(failedRuntime);
    disposed.length = 0;
    expect(JSON.stringify(PM.proj.layers[0])).toBe(layerBefore);
    expect(PM.hist.list()).toEqual(['Replace old.wav']);
    PM.hist.do('Move existing layer', () => { PM.proj.layers[0].from = 7; });
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.layers[0].from).toBe(4);
    PM.proj = JSON.parse(JSON.stringify(PM.proj));
    expect(PM.proj).not.toBe(projectA);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.assets['asset-1']).toMatchObject({ id: 'asset-1', name: 'old.wav', storageKey: 'media:old' });
    expect(PM.assets.get('asset-1')).toBe(oldRuntime);
    expect(JSON.stringify(PM.proj.layers[0])).toBe(layerBefore);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.assets['asset-1'].name).toBe('new.wav');
    expect(PM.assets.get('asset-1')).toBe(newRuntime);

    const thirdRuntime = { id: 'asset-1', name: 'third.wav', kind: 'audio', marker: 'third', persistBlob: new Blob(['third']) };
    PM.Audio.prepareAsset.mockResolvedValueOnce(thirdRuntime);
    await PM.assets.replace('asset-1', { name: 'third.wav', type: 'audio/wav', size: 5 });
    expect(PM.assets.get('asset-1')).toBe(thirdRuntime);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.assets.get('asset-1')).toBe(newRuntime);
    PM.hist.do('New edit discards replacement redo', () => { PM.proj.layers[0].from = 6; });
    expect(disposed).toEqual([thirdRuntime]);

    PM.hist.clear();
    expect(disposed).toEqual([thirdRuntime, oldRuntime]);
    expect(PM.assets.get('asset-1')).toBe(newRuntime);
  });

  it('rejects incompatible, failed, and stale replacements without changing the original asset', async () => {
    let resolvePreparation!: (value: any) => void;
    const preparation = new Promise<any>((resolve) => { resolvePreparation = resolve; });
    const disposed: any[] = [];
    vi.stubGlobal('window', {
      document: { createElement: () => ({ getContext: () => ({}) }) },
      navigator: { hardwareConcurrency: 4 },
      URL: { createObjectURL: () => 'blob:test', revokeObjectURL: vi.fn() },
      powermove: { media: { sourcePath: () => '' } },
      setTimeout, clearTimeout,
    });
  installBridgeForTests((window as any).powermove);
    const PM = makePM('core/history', 'gl/raster');
    const oldRuntime = { id: 'asset-1', name: 'old.wav', kind: 'audio' };
    const projectA: any = {
      id: 'A', assets: { 'asset-1': { id: 'asset-1', name: 'old.wav', kind: 'audio', storageKey: 'media:old' } }, layers: [], comps: {},
    };
    PM.proj = projectA;
    PM.assets.map.set('asset-1', oldRuntime);
    PM.MediaImport = { fingerprint: vi.fn(async () => 'replacement'), storageKeyFor: (value: string) => `media:${value}` };
    PM.MediaStore = { put: vi.fn(async () => true) };
    PM.Audio = {
      accepts: (file: any) => String(file.type).startsWith('audio/'),
      prepareAsset: vi.fn(() => preparation),
      disposeAsset: vi.fn((asset: any) => disposed.push(asset)),
      pause: vi.fn(), rebalanceCache: vi.fn(),
    };
    PM.touch = vi.fn();

    await expect(PM.assets.replace('asset-1', { name: 'wrong.png', type: 'image/png', size: 4 }))
      .rejects.toThrow('Choose an audio file');
    expect(PM.Audio.prepareAsset).not.toHaveBeenCalled();
    expect(PM.hist.list()).toEqual([]);

    const replacing = PM.assets.replace('asset-1', { name: 'new.wav', type: 'audio/wav', size: 4 });
    // The discarded work only exists once decoding has started.
    await vi.waitFor(() => expect(PM.Audio.prepareAsset).toHaveBeenCalled());
    const projectB = { id: 'B', assets: {}, layers: [], comps: {} };
    PM.proj = projectB;
    PM.assets.clear();
    const prepared = { id: 'asset-1', name: 'new.wav', kind: 'audio' };
    resolvePreparation(prepared);

    await expect(replacing).rejects.toThrow(/switched projects/);
    expect(projectA.assets['asset-1'].name).toBe('old.wav');
    expect(projectB.assets).toEqual({});
    expect(PM.hist.list()).toEqual([]);
    expect(disposed).toContain(prepared);
  });
});


it('restores OBJ assets beyond 64 MB without marking their geometry missing', async () => {
  const text = '#' + ' '.repeat(64 * 1024 * 1024) + '\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n';
  const blob = new Blob([text], { type: 'model/obj' });
  const project = { assets: { model: { id: 'model', name: 'large.obj', kind: 'model' } } };
  const PM = rasterRegistry({
    proj: project,
    MediaStore: { get: async () => blob },
    MediaImport: { mapBounded: async (items: any[], _concurrency: number, fn: (item: any) => Promise<any>) => Promise.all(items.map(fn)) },
  } as any);
  await PM.assets.restoreProject(project);
  expect(PM.assets.get('model')?.mesh.triangleCount).toBe(1);
  expect(PM.assets.get('model')?.size).toBe(blob.size);
});

describe('responsive media batches', () => {
  function batch() {
    const PM = rasterRegistry({
      proj: { id: 'original', assets: {}, layers: [], comps: {} },
      uid: (() => { let n = 0; return () => `a${++n}`; })(),
      touch: vi.fn(), bus: { emit: vi.fn(), on: vi.fn() },
    });
    Object.assign(window, { navigator: { hardwareConcurrency: 4 },
      URL: { revokeObjectURL: vi.fn() } });
    installMedia(PM);
    PM.MediaStore.put = vi.fn(async () => true);
    PM.Audio = { accepts: () => true, rebalanceCache: vi.fn(), disposeAsset: vi.fn(),
      prepareAsset: vi.fn(async ({ id, name, blob }: any) => ({ id, name, kind: 'audio', audioBlob: blob, dur: 1, size: blob.size })),
    };
    const file = (name: string) => Object.assign(new Blob([name], { type: 'audio/wav' }), { name });
    return { PM, file };
  }

  it('keeps order and partial success when one decode fails, without storing failed bytes', async () => {
    const { PM, file } = batch();
    const prepare = PM.Audio.prepareAsset.getMockImplementation();
    PM.Audio.prepareAsset.mockImplementation((options: any) => options.name === 'bad.wav'
      ? Promise.reject(new Error('Damaged audio')) : prepare(options));
    let heartbeat = false;
    setTimeout(() => { heartbeat = true; }, 0);
    const results = await PM.assets.importBatch([file('one.wav'), file('bad.wav'), file('three.wav')]);
    expect(heartbeat).toBe(true);
    expect(results.map((r: any) => r.status)).toEqual(['created', 'failed', 'created']);
    expect(results[1].error.message).toBe('Damaged audio');
    expect(Object.values(PM.proj.assets).map((a: any) => a.name).sort()).toEqual(['one.wav', 'three.wav']);
    expect(PM.MediaStore.put).toHaveBeenCalledTimes(2);
  });

  it('cancels preparation before storage, disposes runtime and allows a later import', async () => {
    const { PM, file } = batch(), controller = new AbortController();
    const prepare = PM.Audio.prepareAsset.getMockImplementation();
    PM.Audio.prepareAsset.mockImplementationOnce(async (options: any) => {
      controller.abort(); return prepare(options);
    });
    const results = await PM.assets.importBatch([file('one.wav'), file('two.wav')], { concurrency: 1, signal: controller.signal });
    expect(results.map((r: any) => r.status)).toEqual(['cancelled', 'cancelled']);
    expect(PM.Audio.disposeAsset).toHaveBeenCalledOnce();
    expect(PM.MediaStore.put).not.toHaveBeenCalled();
    expect(PM.assets.map.size).toBe(0);
    expect(PM.proj.assets).toEqual({});
    const retry = await PM.assets.importBatch([file('retry.wav')]);
    expect(retry[0].status).toBe('created');
  });

  it('never starts later files in a newly selected project', async () => {
    const { PM, file } = batch();
    const original = PM.proj, next = { id: 'next', assets: {}, layers: [], comps: {} };
    const results = await PM.assets.importBatch([file('one.wav'), file('two.wav')], {
      concurrency: 1, onProgress: () => { PM.proj = next; },
    });
    expect(results.map((r: any) => r.status)).toEqual(['created', 'failed']);
    expect(results[1].error.message).toMatch(/switched projects/);
    expect(Object.keys(original.assets)).toHaveLength(1);
    expect(next.assets).toEqual({});
    expect(PM.Audio.prepareAsset).toHaveBeenCalledOnce();
  });
});
