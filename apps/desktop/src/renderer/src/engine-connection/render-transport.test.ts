// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { EngineCompatAsk, EngineSnapshotReply, EngineSnapshotRequest } from '../../../shared/engine-ipc';
import type { ShaderFallbackRequest } from '../../../shared/engine-protocol';
import { BLENDER_WAIT_MS, EngineRenderTransport, EngineSnapshotError, type TransportSession, type TransportSvgRasters, svgUploadKey, unpremultiply } from './render-transport';

const quality = { resolution: 'full', representation: 'original', draft3d: false, cached: false, standIn: false } as const;

const rendered = (width: number, height: number, fill = 7, revision = 12): EngineSnapshotReply => ({
  status: 'rendered', revision, viewSeq: 4, width, height, pixels: 'opaque', data: new Uint8Array(width * height * 4).fill(fill), origin: [0, 0], layerBox: null, raster: null, quality, waitedUs: 1, renderUs: 2, mainMs: 3
});

const fallbackRequest = (key: string): ShaderFallbackRequest => ({ key, layer: 'sky', vertex: 'v', fragment: 'f', width: 2, height: 2, calls: [], stage: 'bind', message: '', textures: 0, units: [], morePasses: [] });

function session(): TransportSession & { uploads: string[]; posted: Record<string, unknown>[] } {
  const uploads: string[] = [];
  const posted: Record<string, unknown>[] = [];
  return { id: 5, closed: false, generation: 2, resyncPending: false, uploads, posted, uploadFallback: (p) => { uploads.push(p.key); }, post: (e) => { posted.push(e); return true; } };
}

function canvasFake() {
  const drawn: ImageData[] = [];
  const canvas = {
    width: 0,
    height: 0,
    getContext: () => ({ putImageData: (image: ImageData) => drawn.push(image) }),
    toDataURL: (type: string, quality: number) => `data:${type};q=${quality};${canvas.width}x${canvas.height}`
  };
  return { canvas: canvas as unknown as HTMLCanvasElement, drawn };
}

function setup(replies: EngineSnapshotReply[], options: { flush?: () => number; session?: TransportSession | null; windowDocument?: () => { id: string; revision: number; full(): { json: Uint8Array; hash: string; revision: number } } | null; wake?: () => Promise<void>; usesBlender?: () => boolean; svgRasters?: TransportSvgRasters; captionsHidden?: () => boolean } = {}) {
  const asked: EngineSnapshotRequest[] = [];
  const s = session();
  const runner = { renderSync: vi.fn((r: ShaderFallbackRequest) => ({ key: r.key, width: 2, height: 2, rgba16f: new Uint16Array(16) })), render: vi.fn(async (r: ShaderFallbackRequest) => ({ key: r.key, width: 2, height: 2, rgba16f: new Uint16Array(16) })) };
  let askListener: ((ask: EngineCompatAsk) => void) | null = null;
  const answers: unknown[] = [];
  const fake = canvasFake();
  const transport = new EngineRenderTransport({
    native: {
      snapshotSync: (request) => { asked.push(JSON.parse(JSON.stringify(request))); return replies.shift() ?? { status: 'failed', code: 'internal', message: 'no more replies' }; },
      snapshot: async (request) => { asked.push(JSON.parse(JSON.stringify(request))); return replies.shift() ?? { status: 'failed', code: 'internal', message: 'no more replies' }; },
      onCompatAsk: (listener) => { askListener = listener; return () => { askListener = null; }; },
      answerCompat: (answer) => answers.push(answer)
    },
    session: () => ('session' in options ? options.session! : s),
    revision: options.flush ?? (() => 12),
    windowDocument: options.windowDocument,
    wake: options.wake,
    usesBlender: options.usesBlender,
    captionsHidden: options.captionsHidden,
    composition: () => 'comp-1',
    document: () => 'doc-1',
    viewSeq: () => 4,
    fallbackRunner: () => runner,
    svgRasters: () => options.svgRasters ?? null,
    log: () => {},
    createCanvas: () => fake.canvas
  });
  return { transport, asked, session: s, runner, ask: (a: EngineCompatAsk) => askListener!(a), answers, fake };
}

describe('EngineRenderTransport', () => {
  it('asks synchronously at the flushed revision, with the window session and view', () => {
    let flushed = 0;
    const { transport, asked } = setup([rendered(4, 2)], { flush: () => { flushed++; return 12; } });
    const snap = transport.snapshotSync({ time: 0.5, width: 4, height: 2 });
    expect(flushed).toBe(1);
    expect(asked[0]).toMatchObject({ generation: 2, session: 5, revision: 12, snapshot: { composition: 'comp-1', time: 352_800_000, outputSize: { width: 4, height: 2 }, class: 'sync', viewSeq: 4, pixels: 'opaque', uploaded: [] } });
    expect(snap).toMatchObject({ width: 4, height: 2, revision: 12, viewSeq: 4 });
    expect(snap.data).toBeInstanceOf(Uint8ClampedArray);
  });

  it('draws what the engine needs on this thread and asks again, never waiting on a task', () => {
    const { transport, asked, session: s, runner } = setup([
      { status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('a'.repeat(64))], compat: [] },
      rendered(2, 2)
    ]);
    const snap = transport.snapshotSync({ time: 0, width: 2, height: 2 });
    expect(snap.width).toBe(2);
    expect(runner.renderSync).toHaveBeenCalledOnce();
    expect(runner.render).not.toHaveBeenCalled();
    expect(s.uploads).toEqual(['a'.repeat(64)]);
    expect(asked[1]!.snapshot.uploaded).toEqual(['a'.repeat(64)]);
  });

  it('draws an effect with the textures main handed back with the answer, on this thread and asynchronously', async () => {
    const effect: ShaderFallbackRequest = { ...fallbackRequest('d'.repeat(64)), textures: 2, units: [{ unit: 0, texture: 0, chain: false }, { unit: 1, texture: 1, chain: false }], morePasses: [] };
    // 2 × 2 binary16 RGBA: 32 bytes a texture, at an odd offset as IPC may give them.
    const band = (value: number) => {
      const backing = new Uint8Array(33);
      const bytes = backing.subarray(1);
      new DataView(bytes.buffer, 1).setUint16(0, value, true);
      return bytes;
    };
    const needs = (): EngineSnapshotReply => ({ status: 'needsRenderer', revision: 12, fallback: [effect], compat: [], fallbackTextures: [{ key: effect.key, textures: [band(0x3c00), band(0x3800)] }] });
    const sync = setup([needs(), rendered(2, 2)]);
    sync.transport.snapshotSync({ time: 0, width: 2, height: 2 });
    const [request, textures] = sync.runner.renderSync.mock.calls[0] as unknown as [ShaderFallbackRequest, Uint16Array[]];
    expect(request.key).toBe(effect.key);
    expect(textures.map((t) => [t.length, t[0]])).toEqual([[16, 0x3c00], [16, 0x3800]]);
    expect(sync.session.uploads).toEqual([effect.key]);
    const later = setup([needs(), rendered(2, 2)]);
    await later.transport.snapshot({ time: 0, width: 2, height: 2 });
    expect((later.runner.render.mock.calls[0] as unknown as [ShaderFallbackRequest, Uint16Array[]])[1].map((t) => t[0])).toEqual([0x3c00, 0x3800]);
  });

  it('fails an effect whose textures did not come with the answer rather than drawing it unsampled', () => {
    const effect: ShaderFallbackRequest = { ...fallbackRequest('e'.repeat(64)), textures: 1, units: [{ unit: 0, texture: 0, chain: false }], morePasses: [] };
    const { transport, runner } = setup([{ status: 'needsRenderer', revision: 12, fallback: [effect], compat: [] }]);
    expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(EngineSnapshotError);
    expect(runner.renderSync).not.toHaveBeenCalled();
  });

  it('draws the level 1 SVG rasters a snapshot needs, posts them, and names them in the next ask', async () => {
    const svg = { key: { content: 'c'.repeat(64), width: 3, height: 2 }, asset: 'art' };
    const posted: unknown[] = [];
    const svgRasters: TransportSvgRasters = {
      drawSync: vi.fn(() => new Uint8Array(24)),
      draw: vi.fn(async () => new Uint8Array(24)),
      post: vi.fn((target, key, data) => { posted.push({ key, bytes: data.byteLength }); return target.post({ type: 'compat', compat: 'svgPixels', key, row: 0, rows: key.height, data }); })
    };
    const needs: EngineSnapshotReply = { status: 'needsRenderer', revision: 12, fallback: [], compat: [], svg: [svg] };
    const sync = setup([needs, rendered(2, 2)], { svgRasters });
    expect(sync.transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
    expect(svgRasters.drawSync).toHaveBeenCalledWith(svg);
    expect(sync.session.posted[0]).toMatchObject({ compat: 'svgPixels', key: svg.key });
    expect(sync.asked[1]!.snapshot.uploaded).toEqual([svgUploadKey(svg.key)]);
    expect(svgUploadKey(svg.key)).toBe(`svg:${'c'.repeat(64)}:3x2`);
    // Async snapshots draw asynchronously; a repeated ask is a failure.
    const later = setup([needs, rendered(2, 2)], { svgRasters });
    await later.transport.snapshot({ time: 0, width: 2, height: 2 });
    expect(svgRasters.draw).toHaveBeenCalledWith(svg);
    const repeat = setup([needs, needs], { svgRasters });
    expect(() => repeat.transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(/asked again for an SVG raster/);
    // Without a rasterizer the snapshot fails by name.
    expect(() => setup([needs]).transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(/draws no SVG rasters/);
  });

  it('gives up after bounded rounds and when the engine repeats a program', () => {
    const needs: EngineSnapshotReply = { status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('b'.repeat(64))], compat: [] };
    const { transport } = setup([needs, needs]);
    expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(/asked again/);
    expect(transport.stats.failed).toBe(1);
  });

  it("returns today's shapes: a canvas, a JPEG data URL at 0.74, '' only on failure", () => {
    const { transport, asked, fake } = setup([rendered(480, 270), rendered(320, 180), { status: 'failed', code: 'unsupported', message: 'x' }]);
    expect(transport.snapshotDataUrlSync(1, 480, { w: 1920, h: 1080 })).toBe('data:image/jpeg;q=0.74;480x270');
    expect(asked[0]!.snapshot).toMatchObject({ outputSize: { width: 480, height: 270 }, target: { kind: 'frame', motionBlurSamples: 0 } });
    const canvas = transport.renderFrameToSync(1, 320, 180);
    expect(canvas).toBe(fake.canvas);
    expect(fake.drawn.at(-1)!.width).toBe(320);
    // renderFrameTo keeps today's motion blur default (16 samples).
    expect(asked[1]!.snapshot.target).toMatchObject({ kind: 'frame', motionBlurSamples: 16 });
    expect(transport.snapshotDataUrlSync(1, 480, { w: 1920, h: 1080 })).toBe('');
    expect(transport.stats.lastFailure).toBe('unsupported: x');
  });

  it('carries caption visibility on each sync and async frame without leaking it', async () => {
    const { transport, asked } = setup([rendered(2, 2), rendered(2, 2), rendered(2, 2)]);
    transport.snapshotDataUrlSync(0, 2, { w: 2, h: 2 }, { captionsHidden: true });
    await transport.snapshotDataUrl(0, 2, { w: 2, h: 2 }, { captionsHidden: true });
    transport.snapshotSync({ time: 0, width: 2, height: 2 });
    expect(asked.map(r => r.snapshot.captionsHidden)).toEqual([true, true, false]);
  });

  it('inherits the current app caption flag for general frame captures, with per-request overrides', () => {
    let hidden = true;
    const { transport, asked } = setup(Array.from({ length: 4 }, () => rendered(2, 2)), { captionsHidden: () => hidden });
    transport.renderFrameToSync(0, 2, 2);
    hidden = false;
    transport.snapshotDataUrlSync(0, 2, { w: 2, h: 2 });
    transport.snapshotSync({ time: 0, width: 2, height: 2, captionsHidden: true });
    hidden = true;
    transport.snapshotSync({ time: 0, width: 2, height: 2, captionsHidden: false });
    expect(asked.map(r => r.snapshot.captionsHidden)).toEqual([true, false, true, false]);
  });

  it('fails clearly without a session, and opens one on demand (never at boot)', async () => {
    const { transport, session: s } = setup([rendered(2, 2)]);
    const options = (transport as unknown as { options: { session: () => TransportSession | null; wake?: () => Promise<void> } }).options;
    let open = false;
    let woken = 0;
    options.session = () => (open ? s : null);
    options.wake = async () => { woken++; open = true; };
    // A blocked thread cannot wait: it fails now and the session opens for the next call.
    expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(EngineSnapshotError);
    expect(woken).toBe(1);
    open = false;
    // An async snapshot opens it and goes on.
    expect((await transport.snapshot({ time: 0, width: 2, height: 2 })).width).toBe(2);
    expect(woken).toBe(2);
  });

  it('async siblings draw with the async runner', async () => {
    const { transport, runner } = setup([{ status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('c'.repeat(64))], compat: [] }, rendered(2, 2)]);
    const snap = await transport.snapshot({ time: 0, width: 2, height: 2 });
    expect(snap.width).toBe(2);
    expect(runner.render).toHaveBeenCalledOnce();
    expect(runner.renderSync).not.toHaveBeenCalled();
  });

  it('thumbnails reject stale results and superseded requests', async () => {
    const { transport, asked } = setup([rendered(2, 2, 1, 11), rendered(2, 2, 2, 12), rendered(2, 2, 3, 12)]);
    // Older than the revision the consumer requires.
    expect(await transport.thumbnails.request({ key: 'comp-1', time: 0, width: 2, height: 2, minRevision: 12 })).toBeNull();
    expect(asked[0]!.snapshot.class).toBe('idle');
    // A newer request for the same key replaces the older one.
    const first = transport.thumbnails.request({ key: 'comp-1', time: 0, width: 2, height: 2 });
    const second = transport.thumbnails.request({ key: 'comp-1', time: 0, width: 2, height: 2 });
    expect(await first).toBeNull();
    expect((await second)!.data[0]).toBe(3);
    expect(transport.thumbnails.stale).toBe(2);
  });

  it('thumbnails require the revision the window had when it asked, by default', async () => {
    const { transport } = setup([rendered(2, 2, 1, 12), rendered(2, 2, 2, 13)], { flush: () => 13 });
    expect(await transport.thumbnails.request({ key: 'k', time: 0, width: 2, height: 2 })).toBeNull();
    expect((await transport.thumbnails.request({ key: 'k', time: 0, width: 2, height: 2 }))!.data[0]).toBe(2);
  });

  it('only async captures that prepare their Blender frame wait for Blender, and only in Blender documents', async () => {
    let blender = true;
    const replies = Array.from({ length: 8 }, () => rendered(320, 180));
    const { transport, asked } = setup(replies, { usesBlender: () => blender });
    // Today's snapshotAsync: Blender renders the frame first.
    await transport.snapshotDataUrl(0, 320, { w: 640, h: 360 }, { prepareBlender: true });
    expect(asked[0]!.snapshot).toMatchObject({ class: 'async', blenderWaitMs: BLENDER_WAIT_MS, target: { kind: 'frame', draft3d: false } });
    // Today's synchronous capture shape over invoke (project.snapshot): no.
    await transport.snapshotDataUrl(0, 320, { w: 640, h: 360 });
    expect(asked[1]!.snapshot.blenderWaitMs).toBeUndefined();
    // A blocked thread never waits for Blender.
    transport.snapshotSync({ time: 0, width: 320, height: 180, prepareBlender: true });
    expect(asked[2]!.snapshot.blenderWaitMs).toBeUndefined();
    // Draft 3D draws native shading; nothing to prepare.
    await transport.snapshot({ time: 0, width: 320, height: 180, prepareBlender: true, draft3d: true });
    expect(asked[3]!.snapshot.blenderWaitMs).toBeUndefined();
    // A document without Blender compositions never asks main to wait for it.
    blender = false;
    await transport.snapshotDataUrl(0, 320, { w: 640, h: 360 }, { prepareBlender: true });
    expect(asked[4]!.snapshot.blenderWaitMs).toBeUndefined();
  });

  it('flushes before choosing the path: a flush that starts a staged resync goes to main’s session', () => {
    const live = { ...session(), applied: 12 };
    const windowDocument = () => ({ id: 'doc-1', revision: 13, full: () => ({ json: new TextEncoder().encode('{}'), hash: 'cd'.repeat(32), revision: 13 }) });
    let flushed = 0;
    const { transport, asked } = setup([rendered(2, 2)], { session: live, windowDocument, flush: () => { flushed++; live.resyncPending = true; return 13; } });
    transport.snapshotSync({ time: 0, width: 2, height: 2 });
    expect(flushed).toBe(1);
    expect(asked[0]).toMatchObject({ session: null, revision: 13, document: { id: 'doc-1', revision: 13 } });
  });

  it('uploads compatibility pixels in bands and answers export asks for its document', async () => {
    const { transport, session: s, ask, answers } = setup([]);
    expect(transport.uploadCompat({ layer: 'L7', time: 1, source: 'rasterOverride', key: 'k', width: 2, height: 2, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1, data: new Uint8Array(16) })).toBe(true);
    expect(s.posted[0]).toMatchObject({ type: 'compat', compat: 'pixels', id: { composition: 'comp-1', layer: 'L7', time: 705_600_000, source: 'rasterOverride', key: 'k' }, revision: 12, row: 0, rows: 2 });
    expect(() => transport.uploadCompat({ layer: 'L7', time: 1, source: 'rasterOverride', key: 'k', width: 2, height: 2, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1, data: new Uint8Array(3) })).toThrow();
    const request = { document: 'doc-1', composition: 'comp-1', layer: 'L7', time: 0, source: 'rasterOverride' as const, scale: 1, revision: 12 };
    ask({ ask: 1, request });
    await Promise.resolve();
    expect(answers[0]).toMatchObject({ ask: 1, status: 'unavailable' });
    transport.provideCompat('rasterOverride', () => ({ key: 'k', width: 1, height: 1, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1, data: new Uint8Array(4) }));
    ask({ ask: 2, request });
    ask({ ask: 3, request: { ...request, document: 'other' } });
    await new Promise((r) => setTimeout(r, 0));
    const byAsk = (n: number) => answers.find((a) => (a as { ask: number }).ask === n);
    expect(byAsk(2)).toMatchObject({ status: 'pixels', key: 'k' });
    expect(byAsk(3)).toMatchObject({ status: 'unavailable' });
  });

  it('layer rasters: pixels with anchor and box, null when the layer draws nothing', () => {
    const raster: EngineSnapshotReply = { ...rendered(3, 2), pixels: 'premultiplied', origin: [4, 5], layerBox: [3, 2] } as EngineSnapshotReply;
    const { transport, asked } = setup([raster, { status: 'empty', revision: 12 }, { status: 'failed', code: 'unsupported', message: 'not text' }]);
    const surface = transport.rasterSync('T1', 2, 0.5, { x: 0, y: 0, width: 3, height: 2 });
    expect(surface).toMatchObject({ width: 3, height: 2, origin: [4, 5], layerBox: [3, 2] });
    expect(asked[0]!.snapshot).toMatchObject({ target: { kind: 'layer', layer: 'T1', scale: 2, crop: { x: 0, y: 0, width: 3, height: 2 } }, pixels: 'premultiplied', outputSize: { width: 6000, height: 6000 } });
    expect(transport.rasterSync('T1', 1, 0.5)).toBeNull();
    expect(() => transport.rasterSync('S1', 1, 0.5)).toThrow(EngineSnapshotError);
    // An empty raster is not a failure.
    expect(transport.stats.failed).toBe(1);
  });

  it('a Blender composition without its prepared frame throws today’s error, never an empty snapshot', () => {
    const words = 'The Blender frame is not prepared. Render this frame before capturing it.';
    const { transport } = setup([{ status: 'failed', code: 'blenderFailed', message: words }, { status: 'failed', code: 'blenderFailed', message: words }, { status: 'failed', code: 'timeout', message: 'slow' }]);
    expect(() => transport.snapshotDataUrlSync(0, 320, { w: 640, h: 360 })).toThrow(words);
    expect(() => transport.renderFrameToCanvasSync(0, 64, 36)).toThrow(EngineSnapshotError);
    // Other failures keep today's '' for render.snapshot.
    expect(transport.snapshotDataUrlSync(0, 320, { w: 640, h: 360 })).toBe('');
  });

  describe('sync snapshots the window’s session cannot serve go to main’s session with the document', () => {
    const doc = () => {
      let serialized = 0;
      return {
        get serialized() { return serialized; },
        windowDocument: () => ({ id: 'doc-1', revision: 12, full: () => { serialized++; return { json: new TextEncoder().encode('{"a":1}'), hash: 'ab'.repeat(32), revision: 12 }; } })
      };
    };

    it('a staged resync in flight: the document goes along once, then only its revision', () => {
      const d = doc();
      const pending = { ...session(), resyncPending: true };
      const { transport, asked } = setup([rendered(2, 2), rendered(2, 2)], { session: pending, windowDocument: d.windowDocument });
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect(asked[0]).toMatchObject({ session: null, revision: 12, document: { id: 'doc-1', revision: 12, hash: 'ab'.repeat(32) } });
      expect((asked[0] as any).document.json).toBeTruthy();
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect(asked[1]!.document).toEqual({ id: 'doc-1', revision: 12 });
      expect(d.serialized).toBe(1);
      expect(transport.stats.documentSync).toBe(2);
    });

    it('no session yet (boot): pixels, not \'\', and the standby wakes for the next call', () => {
      const d = doc();
      const wake = vi.fn(async () => {});
      const { transport, asked } = setup([rendered(2, 2)], { session: null, windowDocument: d.windowDocument, wake });
      expect(transport.snapshotDataUrlSync(0, 2, { w: 2, h: 2 })).toMatch(/^data:image\/jpeg/);
      expect(asked[0]).toMatchObject({ session: null });
      expect(wake).toHaveBeenCalled();
    });

    it('main without that revision (an engine restart) gets the document on the next round', () => {
      const d = doc();
      const pending = { ...session(), resyncPending: true };
      const { transport, asked } = setup([rendered(2, 2), { status: 'failed', code: 'needsDocument', message: 'send it' }, rendered(2, 2)], { session: pending, windowDocument: d.windowDocument });
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect(asked[1]!.document!.json).toBeUndefined();
      expect(asked[2]!.document!.json).toBeTruthy();
      expect(d.serialized).toBe(2);
    });

    it('programs only this renderer draws go along with the next round', () => {
      const d = doc();
      const { transport, asked, runner } = setup([
        { status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('c'.repeat(64))], compat: [] },
        rendered(2, 2)
      ], { session: null, windowDocument: d.windowDocument });
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect(runner.renderSync).toHaveBeenCalledTimes(1);
      expect(asked[1]).toMatchObject({ session: null, snapshot: { uploaded: ['c'.repeat(64)] } });
      expect((asked[1] as any).fallbackPixels[0]).toMatchObject({ key: 'c'.repeat(64), width: 2, height: 2 });
    });

    it('level 1 SVG rasters go along with the next round, with no session of the window\'s (boot)', () => {
      const d = doc();
      const svg = { key: { content: 'c'.repeat(64), width: 3, height: 2 }, asset: 'art' };
      const svgRasters: TransportSvgRasters = { drawSync: vi.fn(() => new Uint8Array(24).fill(7)), draw: vi.fn(async () => new Uint8Array(24)), post: vi.fn(() => true) };
      const { transport, asked } = setup([
        { status: 'needsRenderer', revision: 12, fallback: [], compat: [], svg: [svg] },
        rendered(2, 2)
      ], { session: null, windowDocument: d.windowDocument, svgRasters });
      expect(transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
      expect(svgRasters.drawSync).toHaveBeenCalledWith(svg);
      expect(svgRasters.post).not.toHaveBeenCalled();
      expect(asked[0]!.svgPixels).toBeUndefined();
      expect(asked[1]).toMatchObject({ session: null, snapshot: { uploaded: [svgUploadKey(svg.key)] } });
      // (`asked` holds JSON copies: the bytes as an index map.)
      expect(asked[1]!.svgPixels!.map((p) => ({ key: p.key, data: Object.values(p.data) }))).toEqual([{ key: svg.key, data: Array(24).fill(7) }]);
    });

    it('a session with the document and no resync on its way serves it directly', () => {
      const d = doc();
      const { transport, asked } = setup([rendered(2, 2)], { session: { ...session(), applied: 12 }, windowDocument: d.windowDocument });
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect(asked[0]).toMatchObject({ session: 5 });
      expect(asked[0]!.document).toBeUndefined();
      expect(d.serialized).toBe(0);
    });
  });

  it('unpremultiplies like a canvas', () => {
    expect([...unpremultiply(new Uint8ClampedArray([128, 0, 0, 128, 9, 9, 9, 0]))]).toEqual([255, 0, 0, 128, 0, 0, 0, 0]);
  });
});
