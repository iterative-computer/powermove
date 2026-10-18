// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { EngineCompatAsk, EngineSnapshotReply, EngineSnapshotRequest } from '../../../shared/engine-ipc';
import type { ShaderFallbackRequest } from '../../../shared/engine-protocol';
import { EngineRenderTransport, EngineSnapshotError, type TransportSession, unpremultiply } from './render-transport';

const quality = { resolution: 'full', representation: 'original', draft3d: false, cached: false, standIn: false } as const;

const rendered = (width: number, height: number, fill = 7, revision = 12): EngineSnapshotReply => ({
  status: 'rendered', revision, viewSeq: 4, width, height, pixels: 'opaque', data: new Uint8Array(width * height * 4).fill(fill), origin: [0, 0], layerBox: null, raster: null, quality, waitedUs: 1, renderUs: 2, mainMs: 3
});

const fallbackRequest = (key: string): ShaderFallbackRequest => ({ key, layer: 'sky', vertex: 'v', fragment: 'f', width: 2, height: 2, calls: [], stage: 'bind', message: '' });

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

function setup(replies: EngineSnapshotReply[], options: { flush?: () => number } = {}) {
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
    session: () => s,
    revision: options.flush ?? (() => 12),
    composition: () => 'comp-1',
    document: () => 'doc-1',
    viewSeq: () => 4,
    fallbackRunner: () => runner,
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

  it('unpremultiplies like a canvas', () => {
    expect([...unpremultiply(new Uint8ClampedArray([128, 0, 0, 128, 9, 9, 9, 0]))]).toEqual([255, 0, 0, 128, 0, 0, 0, 0]);
  });
});
