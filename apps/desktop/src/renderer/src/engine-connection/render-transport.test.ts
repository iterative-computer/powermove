// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { EngineCompatAsk, EngineSnapshotReply, EngineSnapshotRequest } from '../../../shared/engine-ipc';
import type { ShaderFallbackRequest, SnapshotViewer } from '../../../shared/engine-protocol';
import { BLENDER_WAIT_MS, COMPAT_LOOKAHEAD, intoLoop, runAhead, COMPAT_PREPARE_IN_FLIGHT, EngineRenderTransport, EngineSnapshotError, VANISHING_RASTER_SCALE, type TransportSession, type CompatPrepareOptions, layerRasterScale, unpremultiply } from './render-transport';

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

function setup(replies: EngineSnapshotReply[], options: { flush?: () => number; session?: TransportSession | null; windowDocument?: () => { id: string; revision: number; nativeKernel?: true; full(): { json: Uint8Array; hash: string; revision: number } } | null; wake?: () => Promise<void>; usesBlender?: () => boolean; captionsHidden?: () => boolean } = {}) {
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
    log: () => {},
    createCanvas: () => fake.canvas
  });
  return { transport, asked, session: s, runner, ask: (a: EngineCompatAsk) => askListener!(a), answers, fake };
}

describe('EngineRenderTransport', () => {
  it('pins viewer camera, crop and selected sequence across asynchronous renderer rounds', async () => {
    const viewer: SnapshotViewer = { viewSeq: 19,
      canvas: { width: 2, height: 2, crop: { x: 40, y: 20, width: 100, height: 50 }, css: { x: 5, y: 6, width: 200, height: 100 } },
      view: { camera: { view: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -2, -3, -4, 1], projection: { kind: 'orthographic', halfHeight: 3, near: .01, far: 100 } }, shading: 'solid', xray: true, localView: ['cube'] } };
    const selected = structuredClone(viewer);
    const needs: EngineSnapshotReply = { status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('a'.repeat(64))], compat: [] };
    const reply = { ...rendered(2, 2), viewSeq: 19 } as EngineSnapshotReply;
    const { transport, asked } = setup([needs, reply]);
    const pending = transport.snapshot({ time: .5, width: 2, height: 2, viewer }, 'idle');
    viewer.viewSeq = 20;
    viewer.canvas.crop.x = 80;
    viewer.view!.camera!.view[12] = 99;
    const result = await pending;
    expect(asked).toHaveLength(2);
    for (const request of asked) expect(request.snapshot).toMatchObject({ viewer: selected, viewSeq: 19 });
    expect(result.viewSeq).toBe(19);
    expect(asked[0]!.snapshot.viewer!.view!.camera).not.toBe(viewer.view!.camera);
  });

  it('rejects a thumbnail tagged with a different selected view sequence', async () => {
    const viewer: SnapshotViewer = { view: null, viewSeq: 19, canvas: { width: 2, height: 2, crop: { x: 0, y: 0, width: 2, height: 2 }, css: { x: 0, y: 0, width: 2, height: 2 } } };
    const { transport } = setup([rendered(2, 2)]);
    expect(await transport.thumbnails.request({ key: 'viewer', time: 0, width: 2, height: 2, viewer })).toBeNull();
    expect(transport.thumbnails.stale).toBe(1);
  });

  it('asks synchronously at the flushed revision, with the window session and view', () => {
    let flushed = 0;
    const { transport, asked } = setup([rendered(4, 2)], { flush: () => { flushed++; return 12; } });
    const snap = transport.snapshotSync({ time: 0.5, width: 4, height: 2 });
    expect(flushed).toBe(1);
    expect(asked[0]).toMatchObject({ generation: 2, session: 5, revision: 12, snapshot: { composition: 'comp-1', time: 352_800_000, outputSize: { width: 4, height: 2 }, class: 'sync', viewSeq: 4, pixels: 'opaque', uploaded: [] } });
    expect(snap).toMatchObject({ width: 4, height: 2, revision: 12, viewSeq: 4 });
    expect(snap.data).toBeInstanceOf(Uint8ClampedArray);
  });

  it('never sends main more asynchronous snapshots than it admits, and waits out a refusal instead of failing', async () => {
    // A large project's idle thumbnails and compatibility rasters filled
    // main's eight slots and an extension's renderFrameToAsync failed with
    // "Too many snapshots are in flight" (C1); today's never fails for that.
    const { transport, asked } = setup([]);
    let inFlight = 0, most = 0, refusals = 1;
    (transport as any).options.native.snapshot = async (request: EngineSnapshotRequest) => {
      asked.push(request);
      inFlight++; most = Math.max(most, inFlight);
      await new Promise((r) => setTimeout(r, 5));
      inFlight--;
      if (refusals-- > 0) return { status: 'failed', code: 'refused', message: 'Too many snapshots are in flight; try again when they finish.' } as EngineSnapshotReply;
      return rendered(4, 2);
    };
    const all = await Promise.all([
      ...Array.from({ length: 12 }, () => transport.snapshot({ time: 0, width: 4, height: 2 }, 'idle')),
      ...Array.from({ length: 6 }, () => transport.snapshot({ time: 1, width: 4, height: 2 })),
    ]);
    expect(all).toHaveLength(18);
    expect(all.every((snap) => snap.width === 4)).toBe(true);
    expect(most).toBeLessThanOrEqual(8);
    expect(asked).toHaveLength(19);
  });

  it('prepares playback base pixels over async IPC at dependency priority, without changing ordinary async snapshots', async () => {
    const { transport, asked } = setup([rendered(2, 2), rendered(2, 2)]);
    const sync = vi.spyOn(transport, 'snapshotSync');
    await transport.rasterForCompat('shape', 2, .5);
    expect(sync).not.toHaveBeenCalled();
    expect(asked[0]!.snapshot.class).toBe('sync');
    await transport.raster('shape', 2, .6);
    expect(asked[1]!.snapshot.class).toBe('async');
    expect(transport.stats.async).toBe(2);
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

  it('gives up after bounded rounds and when the engine repeats a program', () => {
    const needs: EngineSnapshotReply = { status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('b'.repeat(64))], compat: [] };
    const { transport } = setup([needs, needs]);
    expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(/asked again/);
    expect(transport.stats.failed).toBe(1);
  });

  it('finishes successive raster and dependent program phases synchronously and asynchronously', async () => {
    const need = { composition: 'comp-1', layer: 'title', time: 0, source: 'rasterOverride' as const, scale: 1 };
    const replies = (): EngineSnapshotReply[] => [
      { status: 'needsRenderer', revision: 12, fallback: [], compat: [need] },
      ...['a', 'b', 'c', 'd'].map((key): EngineSnapshotReply => ({ status: 'needsRenderer', revision: 12, fallback: [fallbackRequest(key.repeat(64))], compat: [] })),
      rendered(2, 2)
    ];
    for (const async of [false, true]) {
      const { transport, asked, runner } = setup(replies());
      transport.answerCompat('rasterOverride', () => ({ kind: 'native' }));
      const result = async ? await transport.snapshot({ time: 0, width: 2, height: 2 }) : transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect(result.width).toBe(2);
      expect(asked).toHaveLength(6);
      expect(asked[5]!.snapshot.uploaded).toHaveLength(4);
      expect(asked[0]!.snapshot.compatUploaded).toEqual([]);
      expect(asked[1]!.snapshot.compatUploaded).toEqual([need]);
      expect(asked[2]!.snapshot.compatUploaded).toEqual([]);
      expect(async ? runner.render : runner.renderSync).toHaveBeenCalledTimes(4);
    }
  });

  it('an async snapshot prepares its compatibility needs asynchronously and posts the prepared answers', async () => {
    const need = { composition: 'comp-1', layer: 'title', time: 0, source: 'rasterOverride' as const, scale: 1 };
    const { transport, session: s } = setup([{ status: 'needsRenderer', revision: 12, fallback: [], compat: [need] }, rendered(2, 2)]);
    const options: unknown[] = [];
    let prepared = false;
    const prepare = vi.fn(async (_need: unknown, revision: number) => { expect(revision).toBe(12); await Promise.resolve(); prepared = true; });
    transport.answerCompat('rasterOverride', (_need, _revision, o) => { options.push(o); return o?.prepared && prepared ? { kind: 'native' } : null; }, prepare);
    expect((await transport.snapshot({ time: 0, width: 2, height: 2 })).width).toBe(2);
    expect(prepare).toHaveBeenCalledOnce();
    // Only the prepared answer: no synchronous answer ran on the caller's turn.
    expect(options).toEqual([{ prepared: true }]);
    expect(s.posted).toContainEqual(expect.objectContaining({ compat: 'native', layer: 'title' }));
  });

  it('an async snapshot answers synchronously only a need whose prepared answer did not survive', async () => {
    const need = { composition: 'comp-1', layer: 'title', time: 0, source: 'rasterOverride' as const, scale: 1 };
    const { transport, session: s } = setup([{ status: 'needsRenderer', revision: 12, fallback: [], compat: [need] }, rendered(2, 2)]);
    const options: unknown[] = [];
    // A reset dropped the prepared answer (or the preparation failed).
    transport.answerCompat('rasterOverride', (_need, _revision, o) => { options.push(o); return o?.prepared ? null : { kind: 'native' }; }, async () => { throw new Error('reset'); });
    expect((await transport.snapshot({ time: 0, width: 2, height: 2 })).width).toBe(2);
    expect(options).toEqual([{ prepared: true }, undefined]);
    expect(s.posted.filter((p) => p['compat'] === 'native')).toHaveLength(1);
    // A synchronous snapshot keeps answering on the calling thread.
    const sync = setup([{ status: 'needsRenderer', revision: 12, fallback: [], compat: [need] }, rendered(2, 2)]);
    const prepare = vi.fn(async () => {});
    sync.transport.answerCompat('rasterOverride', () => ({ kind: 'native' }), prepare);
    expect(sync.transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
    expect(prepare).not.toHaveBeenCalled();
  });

  it('still stops a repeated unresolved raster dependency after three replies', async () => {
    const need = { composition: 'comp-1', layer: 'title', time: 0, source: 'rasterOverride' as const, scale: 1 };
    const needs: EngineSnapshotReply = { status: 'needsRenderer', revision: 12, fallback: [], compat: [need] };
    for (const async of [false, true]) {
      const { transport, asked } = setup([needs, needs, needs, rendered(2, 2)]);
      transport.answerCompat('rasterOverride', () => ({ kind: 'native' }));
      if (async) await expect(transport.snapshot({ time: 0, width: 2, height: 2 })).rejects.toThrow(/same unresolved snapshot dependencies 3 times/);
      else expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(/same unresolved snapshot dependencies 3 times/);
      expect(asked).toHaveLength(3);
    }
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

  it('an export ask carrying the snapshot\'s layer is drawn from it, in any window with the producer', async () => {
    const { transport, ask, answers } = setup([]);
    const seen: unknown[] = [];
    transport.answerCompat('rasterOverride', (_need, _revision, options) => {
      seen.push(options);
      return { kind: 'raster', key: 'k', width: 1, height: 1, format: 'rgba8Premultiplied', data: new Uint8Array(4), anchor: [0, 0], size: [1, 1] };
    });
    const request = { document: 'queued-project', composition: 'comp-1', layer: 'L7', time: 0, source: 'rasterOverride' as const, scale: 1, revision: 1 };
    ask({ ask: 7, request, layer: '{"id":"L7","type":"text"}' });
    ask({ ask: 8, request });
    await new Promise((r) => setTimeout(r, 0));
    const byAsk = (n: number) => answers.find((a) => (a as { ask: number }).ask === n);
    expect(byAsk(7)).toMatchObject({ status: 'pixels' });
    expect(seen).toEqual([{ fresh: true, layer: '{"id":"L7","type":"text"}' }]);
    // Without the snapshot's layer, only the document's window answers.
    expect(byAsk(8)).toMatchObject({ status: 'unavailable', message: 'This window has another document open.' });
  });

  it('layer rasters: pixels with anchor and box, null when the layer draws nothing', () => {
    const raster: EngineSnapshotReply = { ...rendered(3, 2), pixels: 'premultiplied', origin: [4, 5], layerBox: [3, 2] } as EngineSnapshotReply;
    const { transport, asked } = setup([raster, { status: 'empty', revision: 12 }, { status: 'failed', code: 'unsupported', message: 'not text' }]);
    const surface = transport.rasterSync('T1', 2, 0.5, { x: 0, y: 0, width: 3, height: 2 });
    expect(surface).toMatchObject({ width: 3, height: 2, origin: [4, 5], layerBox: [3, 2] });
    expect(asked[0]!.snapshot).toMatchObject({ target: { kind: 'layer', layer: 'T1', scale: 2, crop: { x: 0, y: 0, width: 3, height: 2 } }, pixels: 'premultiplied', outputSize: { width: 8192, height: 8192 } });
    expect(transport.rasterSync('T1', 1, 0.5)).toBeNull();
    expect(() => transport.rasterSync('S1', 1, 0.5)).toThrow(EngineSnapshotError);
    // An empty raster is not a failure.
    expect(transport.stats.failed).toBe(1);
  });

  it('layer rasters take any scale, as today\'s PM.raster: the engine caps the density at the 8192-pixel edge', () => {
    expect([33, 40, 50, 1000, 0.005, 1e-7].map(layerRasterScale)).toEqual([33, 40, 50, 1000, 0.005, 1e-7]);
    // Above 2^53 every number is an integer the MessagePack encoder refuses.
    for (const s of [Infinity, Number.MAX_VALUE, 1e300, 2 ** 53 + 2]) expect(layerRasterScale(s)).toBe(Number.MAX_SAFE_INTEGER);
    for (const s of [0, -1, -Infinity, NaN]) expect(layerRasterScale(s)).toBe(VANISHING_RASTER_SCALE);
    const raster = { ...rendered(3, 2), pixels: 'premultiplied', origin: [4, 5], layerBox: [3, 2] } as EngineSnapshotReply;
    const { transport, asked } = setup([raster, raster, raster, raster, raster]);
    for (const scale of [40, 1000, Infinity, -2, NaN]) expect(transport.rasterSync('T1', scale, 0)).toMatchObject({ width: 3, height: 2 });
    expect(asked.map((a) => (a.snapshot.target as { scale: number }).scale)).toEqual([40, 1000, Number.MAX_SAFE_INTEGER, VANISHING_RASTER_SCALE, VANISHING_RASTER_SCALE]);
    expect(transport.stats.failed).toBe(0);
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

    it.each(['no session', 'resync pending', 'ready session before crash notification'])('uses only an opaque native reference for every %s snapshot round', (state) => {
      const full = vi.fn(() => { throw new Error('native references must never serialize JSON'); });
      const pending = state === 'no session' ? null : { ...session(), resyncPending: state === 'resync pending' };
      const { transport, asked, runner } = setup([
        { status: 'needsRenderer', revision: 12, fallback: [fallbackRequest('n'.repeat(64))], compat: [] },
        rendered(2, 2),
      ], { session: pending, windowDocument: () => ({ id: 'doc-1', revision: 12, nativeKernel: true, full }) });

      expect(transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
      expect(runner.renderSync).toHaveBeenCalledOnce();
      expect(full).not.toHaveBeenCalled();
      expect(asked.map((ask) => ask.document)).toEqual([
        { id: 'doc-1', revision: 12, nativeKernel: true },
        { id: 'doc-1', revision: 12, nativeKernel: true },
      ]);
      expect(asked.every((ask) => ask.revision === 12 && ask.session === null)).toBe(true);
      expect(asked.every((ask) => !('owner' in ask.document!))).toBe(true);
    });

    it('keeps native references after a main-session needsDocument response', () => {
      const full = vi.fn(() => { throw new Error('native references must never serialize JSON'); });
      const reference = { id: 'doc-1', revision: 12, nativeKernel: true as const };
      const { transport, asked } = setup([
        rendered(2, 2),
        { status: 'failed', code: 'needsDocument', message: 'session restarted' },
        rendered(2, 2),
      ], { session: null, windowDocument: () => ({ ...reference, full }) });

      expect(transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
      expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(EngineSnapshotError);
      expect(transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
      expect(asked.map((ask) => ask.document)).toEqual([reference, reference, reference]);
      expect(full).not.toHaveBeenCalled();
    });

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

    it('main-owned snapshots finish a chain of distinct dependent programs', () => {
      const d = doc();
      const { transport, asked, runner } = setup([
        ...['a', 'b', 'c', 'd'].map((key): EngineSnapshotReply => ({ status: 'needsRenderer', revision: 12, fallback: [fallbackRequest(key.repeat(64))], compat: [] })),
        rendered(2, 2)
      ], { session: null, windowDocument: d.windowDocument });
      expect(transport.snapshotSync({ time: 0, width: 2, height: 2 }).width).toBe(2);
      expect(runner.renderSync).toHaveBeenCalledTimes(4);
      expect(asked).toHaveLength(5);
      expect(asked[4]!.snapshot.uploaded).toHaveLength(4);
      expect((asked[4] as any).fallbackPixels[0].key).toBe('d'.repeat(64));
    });

    it('a raster override: main\'s session learns the producers, and the needs\' answers go along with the next round', () => {
      const d = doc();
      const need = { composition: 'comp-1', layer: 'title', time: 0, source: 'rasterOverride' as const, scale: 1 };
      const { transport, asked } = setup([
        { status: 'needsRenderer', revision: 12, fallback: [], compat: [need] },
        rendered(2, 2)
      ], { session: null, windowDocument: d.windowDocument });
      transport.answerCompat('rasterOverride', () => ({ kind: 'raster', key: 'k', width: 1, height: 1, format: 'rgba8Premultiplied', data: new Uint8Array(4).fill(7), anchor: [0, 0], size: [1, 1] }));
      transport.snapshotSync({ time: 0, width: 2, height: 2 });
      expect((asked[0] as any).compatEvents).toEqual([{ type: 'compat', compat: 'producers', sources: ['rasterOverride'] }]);
      const events = (asked[1] as any).compatEvents as any[];
      expect(events.map((e) => e.compat)).toEqual(['producers', 'pixels']);
      expect(events[1]).toMatchObject({ id: { layer: 'title', key: 'k' }, revision: 12, width: 1, height: 1 });
    });

    it('a document snapshot whose need no producer answers fails by name, never draws without it', () => {
      const d = doc();
      const need = { composition: 'comp-1', layer: 'title', time: 0, source: 'rasterOverride' as const, scale: 1 };
      const { transport } = setup([{ status: 'needsRenderer', revision: 12, fallback: [], compat: [need] }], { session: null, windowDocument: d.windowDocument });
      expect(() => transport.snapshotSync({ time: 0, width: 2, height: 2 })).toThrow(/No raster override in this window answered for layer title/);
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

describe('compatibility answers during playback', () => {
  const frame = 23_520_000;
  function rig(playing: boolean, extra: Record<string, unknown> = {}) {
    const posted: any[] = [];
    const session: TransportSession = { id: 3, closed: false, generation: 2, resyncPending: false, applied: 5, uploadFallback: () => {}, post: (e) => { posted.push(e); return true; } };
    const state = { playing, time: 0, rate: 1, revision: 5 };
    const t = new EngineRenderTransport({
      native: {}, session: () => session, revision: () => state.revision, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {},
      playing: () => state.playing, frameTicks: () => frame, playhead: () => ({ time: state.time, rate: state.rate }), ...extra
    } as any);
    const asked: number[] = [];
    t.answerCompat('rasterOverride', (need) => { asked.push(need.time); return { kind: 'native' }; });
    return { t, session, posted, asked, state };
  }
  const need = (time: number, layer = 'title') => ({ composition: 'P-root', layer, time, source: 'rasterOverride' as const, scale: 1 });
  /** A producer whose answers are ready when `prepare` returns (pass-through layers). */
  function passThrough(t: EngineRenderTransport) {
    const prepared: Array<{ layer: string; time: number; options?: CompatPrepareOptions }> = [];
    const answered: Array<{ layer: string; time: number; prepared?: boolean }> = [];
    t.answerCompat('rasterOverride', (n, _revision, options) => { answered.push({ layer: n.layer, time: n.time, prepared: options?.prepared }); return { kind: 'native' }; },
      (n, _revision, options) => { prepared.push({ layer: n.layer, time: n.time, ...(options ? { options } : {}) }); return null; });
    return { prepared, answered };
  }
  /** A producer whose answers wait for the test to finish them (bases on their way). */
  function waiting(t: EngineRenderTransport) {
    const calls: Array<{ layer: string; time: number; options?: CompatPrepareOptions; done: () => void; fail: (error: Error) => void }> = [];
    const answered: Array<{ layer: string; time: number }> = [];
    t.answerCompat('rasterOverride', n => { answered.push({ layer: n.layer, time: n.time }); return { kind: 'native' }; },
      (n, _revision, options) => new Promise<void>((done, fail) => calls.push({ layer: n.layer, time: n.time, ...(options ? { options } : {}), done, fail })));
    return { calls, answered };
  }
  const layers = ['a', 'b', 'c'];
  const frames = (from: number, count: number, step = 1) => Array.from({ length: count }, (_, i) => (from + i * step) * frame);

  it('prepares whole frames in playback order, every layer of a frame together, and each frame once', () => {
    const { t, session, state } = rig(true);
    const { prepared, answered } = passThrough(t);
    const now = vi.spyOn(performance, 'now').mockReturnValue(0);
    try {
      t.receiveCompat({ compat: 'needs', needs: layers.map(l => need(0, l)) }, session);
      // Frame by frame, each frame's layers in one run, through the horizon.
      expect(prepared.map(p => [p.time / frame, p.layer])).toEqual(frames(0, COMPAT_LOOKAHEAD + 1).flatMap(time => layers.map(l => [time / frame, l])));
      // Every answer is posted as soon as its preparation returns.
      expect(answered).toHaveLength(prepared.length);
      expect(answered.every(a => a.prepared)).toBe(true);
      const count = prepared.length;
      for (let i = 0; i < 20; i++) t.pump();
      expect(prepared).toHaveLength(count);
      // The clock moves on two frames: only the two frames entering the horizon.
      state.time = 2 * frame;
      t.pump();
      expect(prepared.slice(count).map(p => [p.time / frame, p.layer])).toEqual([COMPAT_LOOKAHEAD + 1, COMPAT_LOOKAHEAD + 2].flatMap(n => layers.map(l => [n, l])));
    } finally { now.mockRestore(); t.dispose(); }
  });

  it('keeps a started frame whole and leaves the next frames to a later task when the turn runs out of time', async () => {
    vi.useFakeTimers();
    const { t, session } = rig(true);
    let spent = 0;
    const now = vi.spyOn(performance, 'now').mockImplementation(() => spent);
    const prepared: number[] = [];
    t.answerCompat('rasterOverride', () => ({ kind: 'native' }), n => { prepared.push(n.time); spent += 4; return null; });
    try {
      t.receiveCompat({ compat: 'needs', needs: layers.map(l => need(0, l)) }, session);
      // 12 ms of wrappers for frame 0 exceed the 6 ms turn, but the frame is whole.
      expect(prepared).toEqual(frames(0, 3, 0));
      await vi.advanceTimersByTimeAsync(0);
      expect(prepared.slice(3)).toEqual(frames(1, 3, 0));
      for (let i = 0; i < 40 && prepared.length < 3 * (COMPAT_LOOKAHEAD + 1); i++) await vi.advanceTimersByTimeAsync(1);
      expect(new Set(prepared)).toEqual(new Set(frames(0, COMPAT_LOOKAHEAD + 1)));
      t.dispose();
      const before = prepared.length;
      await vi.advanceTimersByTimeAsync(1000);
      expect(prepared).toHaveLength(before);
    } finally { now.mockRestore(); t.dispose(); vi.useRealTimers(); }
  });

  it('bounds playback preparations waiting on bases, starts frames in order, and posts each answer when it is ready', async () => {
    vi.useFakeTimers();
    const { t, session, state } = rig(true);
    const { calls, answered } = waiting(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0, 'a'), need(0, 'b')] }, session);
      // Whole frames up to the budget: frames 0 to 5 of two layers.
      expect(calls).toHaveLength(COMPAT_PREPARE_IN_FLIGHT);
      expect(calls.map(c => c.time)).toEqual(frames(0, COMPAT_PREPARE_IN_FLIGHT / 2).flatMap(time => [time, time]));
      // Frames the running clock shows: their later turns go ahead of ordinary tasks.
      expect(calls.every(c => c.options?.due && !c.options.urgent)).toBe(true);
      expect(t.lookaheadStats.maxInFlight).toBe(COMPAT_PREPARE_IN_FLIGHT);
      // A later answer does not wait for an earlier slow one.
      calls[3]!.done();
      await vi.advanceTimersByTimeAsync(0);
      expect(answered).toEqual([{ layer: 'b', time: frame }]);
      // Its freed budget starts the next frame, whole.
      expect(calls.slice(COMPAT_PREPARE_IN_FLIGHT).map(c => [c.layer, c.time / frame])).toEqual([['a', COMPAT_PREPARE_IN_FLIGHT / 2], ['b', COMPAT_PREPARE_IN_FLIGHT / 2]]);
      // The clock leaves frame 0 behind: its late answers are not posted, and
      // the budget they free goes to the clock's frames.
      state.time = 30 * frame;
      calls[0]!.done(); calls[1]!.done();
      await vi.advanceTimersByTimeAsync(0);
      expect(answered).toHaveLength(1);
      expect(t.lookaheadStats.obsolete).toBe(2);
      expect(calls.slice(-2).map(c => c.time)).toEqual([30 * frame, 30 * frame]);
    } finally { state.playing = false; t.dispose(); vi.useRealTimers(); }
  });

  it('moves on with the clock once a frame while playing, never as a poll, and stops when playback does', async () => {
    vi.useFakeTimers();
    const { t, session, state } = rig(true);
    const { prepared } = passThrough(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      expect(vi.getTimerCount()).toBe(1);
      const pumps = t.lookaheadStats.pumps;
      await vi.advanceTimersByTimeAsync(100);
      // About three turns in 100 ms at 30 fps.
      expect(t.lookaheadStats.pumps - pumps).toBeLessThanOrEqual(4);
      state.time = 3 * frame;
      await vi.advanceTimersByTimeAsync(34);
      expect(Math.max(...prepared.map(p => p.time))).toBe((3 + COMPAT_LOOKAHEAD) * frame);
      state.playing = false;
      await vi.advanceTimersByTimeAsync(34);
      expect(vi.getTimerCount()).toBe(0);
    } finally { t.dispose(); vi.useRealTimers(); }
  });

  it('prepares only the paused frame on screen, urgently, with no timer, and again after an edit', async () => {
    vi.useFakeTimers();
    const { t, session, state } = rig(false);
    const { prepared, answered } = passThrough(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: layers.map(l => need(4 * frame, l)) }, session);
      expect(prepared).toEqual(layers.map(layer => ({ layer, time: 4 * frame, options: { urgent: true } })));
      expect(answered.map(a => a.layer)).toEqual(layers);
      await vi.advanceTimersByTimeAsync(1000);
      expect(prepared).toHaveLength(3);
      expect(vi.getTimerCount()).toBe(0);
      // A paused edit: the frame on screen at the new revision, nothing else.
      state.revision++;
      t.prepareShown();
      expect(prepared.slice(3)).toEqual(layers.map(layer => ({ layer, time: 4 * frame, options: { urgent: true } })));
      await vi.advanceTimersByTimeAsync(1000);
      expect(prepared).toHaveLength(6);
      // Play from the actual playhead.
      state.time = 12 * frame; state.playing = true;
      t.playbackStarted();
      expect(prepared.slice(6).map(p => p.time)).toEqual(frames(12, COMPAT_LOOKAHEAD + 1).flatMap(time => [time, time, time]));
    } finally { state.playing = false; t.dispose(); vi.useRealTimers(); }
  });

  it('starts the paused frame on screen beside playback preparations holding the budget, and promotes one already making it', async () => {
    vi.useFakeTimers();
    const { t, session, state } = rig(true);
    const { calls, answered } = waiting(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      expect(calls).toHaveLength(COMPAT_PREPARE_IN_FLIGHT);
      state.playing = false;
      // Stopped on frame 2, which playback is still preparing.
      let promoted = false;
      void calls[2]!.options!.promoted!.then(() => { promoted = true; });
      t.receiveCompat({ compat: 'needs', needs: [need(2 * frame)] }, session);
      await vi.advanceTimersByTimeAsync(0);
      expect(promoted).toBe(true);
      expect(calls.filter(c => c.time === 2 * frame)).toHaveLength(1);
      calls[2]!.done();
      await vi.advanceTimersByTimeAsync(0);
      expect(answered).toEqual([{ layer: 'title', time: 2 * frame }]);
      // A seek while every playback slot is held: the frame starts at once.
      t.receiveCompat({ compat: 'needs', needs: [need(40 * frame)] }, session);
      const seek = calls.find(c => c.time === 40 * frame);
      expect(seek?.options).toEqual({ urgent: true });
      seek!.done();
      await vi.advanceTimersByTimeAsync(0);
      expect(answered.at(-1)).toEqual({ layer: 'title', time: 40 * frame });
      // Playback's other completions are not the paused frame's: dropped.
      for (const c of calls) c.done();
      await vi.advanceTimersByTimeAsync(0);
      expect(answered).toHaveLength(2);
      expect(t.lookaheadStats.inFlight).toBe(0);
      expect(vi.getTimerCount()).toBe(0);
    } finally { t.dispose(); vi.useRealTimers(); }
  });

  it('finishes an in-flight playback need when its stopped clock arrives first', async () => {
    vi.useFakeTimers();
    const { t, session, state } = rig(true);
    const { calls, answered } = waiting(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      state.playing = false;
      t.receiveCompat({ compat: 'needs', needs: [need(0), need(0, 'last-layer')] }, session);
      expect(answered).toEqual([]);
      // The playback job for frame zero is now the paused frame's; the other layer starts urgently.
      expect(calls.filter(c => c.layer === 'last-layer').map(c => c.options)).toEqual([{ urgent: true }]);
      for (const c of calls) c.done();
      await vi.advanceTimersByTimeAsync(0);
      expect(answered.map(a => [a.layer, a.time])).toEqual([['title', 0], ['last-layer', 0]]);
      expect(t.lookaheadStats.inFlight).toBe(0);
      expect(vi.getTimerCount()).toBe(0);
    } finally { t.dispose(); vi.useRealTimers(); }
  });

  it('reports a failed preparation once and asks again only on a fresh need', async () => {
    vi.useFakeTimers();
    const errors: unknown[] = [];
    const { t, session } = rig(false, { onProducerError: (error: unknown) => errors.push(error) });
    const { calls } = waiting(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      calls[0]!.fail(new Error('wrapper threw'));
      await vi.advanceTimersByTimeAsync(100);
      expect(errors).toHaveLength(1);
      expect(calls).toHaveLength(1);
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      expect(calls).toHaveLength(2);
    } finally { t.dispose(); vi.useRealTimers(); }
  });

  it('starts a layer whose answers take longer than the clock leaves where it can still be in time', async () => {
    vi.useFakeTimers();
    const { t, session, state } = rig(true);
    const prepared: Array<[string, number]> = [];
    const slow: Array<() => void> = [];
    t.answerCompat('rasterOverride', () => ({ kind: 'native' }), n => {
      prepared.push([n.layer, n.time / frame]);
      return n.layer === 'slow' ? new Promise<void>(resolve => slow.push(resolve)) : null;
    });
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0, 'slow'), need(0, 'fast')] }, session);
      // Each slow preparation takes 100 ms: three frames at 30 fps.
      await vi.advanceTimersByTimeAsync(100);
      while (slow.length) slow.shift()!();
      await vi.advanceTimersByTimeAsync(0);
      prepared.length = 0;
      state.time = 40 * frame;
      t.pump();
      const slowFrames = prepared.filter(([layer]) => layer === 'slow').map(([, n]) => n);
      const fastFrames = prepared.filter(([layer]) => layer === 'fast').map(([, n]) => n);
      // The fast layer from the clock's frame; the slow one from the first it can still make.
      expect(Math.min(...fastFrames)).toBe(40);
      expect(Math.min(...slowFrames)).toBe(43);
      expect(t.lookaheadStats.latePrepareSkips).toBeGreaterThan(0);
      for (let i = 0; i < 10 && slow.length; i++) {
        while (slow.length) slow.shift()!();
        await vi.advanceTimersByTimeAsync(0);
      }
      // A stopped line (a Play held in the engine's preroll) prepares the frames at the clock.
      state.rate = 0; state.time = 80 * frame;
      prepared.length = 0;
      t.pump();
      expect(prepared).toContainEqual(['slow', 80]);
    } finally { state.playing = false; t.dispose(); vi.useRealTimers(); }
  });

  it('recovers from a missed horizon without inferring a reversal from a delayed need', () => {
    const { t, session, state } = rig(true);
    const { prepared } = passThrough(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      const before = prepared.length;
      state.time = 30 * frame;
      // The engine's request can arrive after the audio clock moved on.
      t.receiveCompat({ compat: 'needs', needs: [need(frame)] }, session);
      expect(prepared.slice(before).map(p => p.time)).toEqual(frames(30, COMPAT_LOOKAHEAD + 1));
      expect(t.lookaheadStats.skipped).toBe(1);
      expect(t.lookaheadStats.recoveries).toBeGreaterThan(0);
    } finally { state.playing = false; t.dispose(); }
  });

  it('prepares nothing past the end of the playable span, or outside a layer\'s own time span', () => {
    const state = { time: 80 * frame };
    const { t, session } = rig(true, {
      playhead: () => ({ time: state.time, rate: 1 }), span: () => ({ start: 0, end: 90 * frame }),
      layerSpan: (composition: string, layer: string) => composition === 'P-root' && layer === 'brief' ? { start: 82 * frame, end: 85 * frame } : null
    });
    const { prepared } = passThrough(t);
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(80 * frame), need(80 * frame, 'brief')] }, session);
      // Frames 80 to 89 only: a 90-frame composition never shows frame 90.
      expect(new Set(prepared.filter(p => p.layer === 'title').map(p => p.time))).toEqual(new Set(frames(80, 10)));
      // A layer from 82 to 85 (a frame of slack each side): frames 81 to 85, not the rest.
      expect(prepared.filter(p => p.layer === 'brief').map(p => p.time / frame)).toEqual([81, 82, 83, 84, 85]);
      // A nested composition's times are its own: the open span does not bound them.
      t.receiveCompat({ compat: 'needs', needs: [{ ...need(95 * frame), composition: 'nested' }] }, session);
      expect(prepared.some(p => p.time >= 95 * frame)).toBe(true);
    } finally { t.dispose(); }
  });

  it('keeps a nested composition time offset while following the open clock', () => {
    const { t, session, state } = rig(true);
    const { prepared } = passThrough(t);
    try {
      state.time = 20 * frame;
      t.receiveCompat({ compat: 'needs', needs: [{ ...need(5 * frame), composition: 'nested' }] }, session);
      expect(prepared.at(-1)!.time).toBe((5 + COMPAT_LOOKAHEAD) * frame);
      state.time = 30 * frame;
      t.pump();
      expect(prepared.at(-1)!.time).toBe((15 + COMPAT_LOOKAHEAD) * frame);
    } finally { state.playing = false; t.dispose(); }
  });

  it('uses the explicit reverse rate and starts over on a seek or revision change', () => {
    const { t, session, state } = rig(true);
    const { prepared } = passThrough(t);
    try {
      state.time = 20 * frame; state.rate = -1;
      t.receiveCompat({ compat: 'needs', needs: [need(state.time)] }, session);
      expect(prepared.map(p => p.time)).toEqual(frames(20, COMPAT_LOOKAHEAD + 1, -1));
      state.time = 40 * frame;
      t.pump();
      expect(prepared.at(-1)!.time).toBe((40 - COMPAT_LOOKAHEAD) * frame);
      const before = prepared.length;
      state.revision++;
      t.pump();
      t.pump();
      expect(prepared.length - before).toBe(COMPAT_LOOKAHEAD + 1);
    } finally { state.playing = false; t.dispose(); }
  });

  it('answers the frame on screen again on a paused redraw and says whether the output changed', () => {
    const { t, session, posted } = rig(false);
    let key = 'k1', reuse = false;
    const raster = () => ({ kind: 'raster', key, width: 1, height: 1, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1, data: new Uint8Array(4), anchor: [0, 0], size: [1, 1] } as any);
    t.answerCompat('rasterOverride', () => reuse ? { kind: 'reuse', key } : raster());
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(3 * frame)] }, session);
      const before = posted.length;
      // Unchanged: pixels again under the key already posted, or a reuse.
      expect(t.refreshShown(3 * frame)).toBe(false);
      expect(posted.length).toBeGreaterThan(before);
      reuse = true;
      expect(t.refreshShown(3 * frame)).toBe(false);
      // Its output changed: new pixels under a new key, after the layer's
      // stale answers are forgotten.
      reuse = false; key = 'k2';
      const at = posted.length;
      expect(t.refreshShown(3 * frame)).toBe(true);
      const sent = posted.slice(at).map((m: any) => m.compat);
      expect(sent[0]).toBe('forget');
      expect(sent.slice(1).every((c: string) => c === 'pixels')).toBe(true);
      // The new output again: unchanged.
      expect(t.refreshShown(3 * frame)).toBe(false);
    } finally { t.dispose(); }
  });

  it('measures distances along a looping run: the frames after the wrap follow the last ones', () => {
    const loop = { start: 10 * frame, end: 100 * frame };
    expect(runAhead(98 * frame, 12 * frame, frame, loop)).toBe(4 * frame);
    expect(runAhead(98 * frame, 97 * frame, frame, loop)).toBe(-frame);
    expect(runAhead(12 * frame, 98 * frame, -frame, loop)).toBe(4 * frame);
    // Without a loop, plain distances.
    expect(runAhead(98 * frame, 12 * frame, frame, null)).toBe(-86 * frame);
    expect(intoLoop(101 * frame, loop)).toBe(11 * frame);
    expect(intoLoop(9 * frame, loop)).toBe(99 * frame);
    expect(intoLoop(50 * frame, loop)).toBe(50 * frame);
    expect(intoLoop(101 * frame, null)).toBe(101 * frame);
  });

  it('prepares and answers the frames after a loop\'s wrap before playback reaches them, and not again after it', async () => {
    vi.useFakeTimers();
    const posted: any[] = [];
    const session: TransportSession = { id: 3, closed: false, generation: 2, resyncPending: false, applied: 5, uploadFallback: () => {}, post: (e) => { posted.push(e); return true; } };
    const state = { time: 80 * frame };
    const t = new EngineRenderTransport({
      native: {}, session: () => session, revision: () => 5, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {},
      playing: () => true, frameTicks: () => frame, playhead: () => ({ time: state.time, rate: 1 }), span: () => ({ start: 0, end: 90 * frame, loop: true })
    } as any);
    const prepared: number[] = [];
    const answered: number[] = [];
    t.answerCompat('rasterOverride', n => { answered.push(n.time); return { kind: 'native' }; }, async n => { prepared.push(n.time); });
    try {
      t.receiveCompat({ compat: 'needs', needs: [need(80 * frame)] }, session);
      for (let i = 0; i < 40; i++) await vi.advanceTimersByTimeAsync(4);
      // Frames 80 to 89, then 0 onwards: the clock wraps to the loop's start.
      const horizon = Array.from({ length: COMPAT_LOOKAHEAD + 1 }, (_, i) => ((80 + i) % 90) * frame);
      for (const time of horizon) expect(answered, `frame ${time / frame}`).toContain(time);
      expect(Math.max(...answered)).toBe(89 * frame);
      expect(prepared.every(time => time >= 0 && time < 90 * frame)).toBe(true);
      // The clock reaches the loop's last frame, then wraps: frames already
      // answered are not answered again.
      const before = answered.length;
      state.time = 89 * frame;
      for (let i = 0; i < 10; i++) await vi.advanceTimersByTimeAsync(4);
      state.time = 0;
      for (let i = 0; i < 10; i++) await vi.advanceTimersByTimeAsync(4);
      const again = answered.slice(before);
      expect(again.filter(time => time <= 8 * frame)).toEqual([]);
      expect(again).toContain(COMPAT_LOOKAHEAD * frame);
      // The engine asks for the first frame after the wrap while the clock
      // still shows the last ones: a frame ahead, not a stale one.
      state.time = 89 * frame;
      const skipped = t.lookaheadStats.skipped;
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      expect(t.lookaheadStats.skipped).toBe(skipped);
    } finally { t.dispose(); vi.useRealTimers(); }
  });

  it('remembers the answer posted for each layer and frame, and only once posted', () => {
    let open = true;
    const session: TransportSession = { id: 3, closed: false, generation: 2, resyncPending: false, applied: 5, uploadFallback: () => {}, post: () => open };
    const state = { time: 0, revision: 5 };
    const t = new EngineRenderTransport({
      native: {}, session: () => session, revision: () => state.revision, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {},
      playing: () => false, frameTicks: () => frame, playhead: () => ({ time: state.time, rate: 1 }), span: () => ({ start: 0, end: 90 * frame })
    } as any);
    t.answerCompat('rasterOverride', n => n.time === frame ? { kind: 'native' } : { kind: 'reuse', key: `k${n.time}` });
    try {
      // Paused needs, answered at once.
      t.receiveCompat({ compat: 'needs', needs: [need(0), need(frame)] }, session);
      expect(t.answeredAt('title', 0)).toEqual({ revision: 5, kind: 'reuse' });
      expect(t.answeredAt('title', frame)).toEqual({ revision: 5, kind: 'native' });
      expect(t.answeredAt('other', 0)).toBeNull();
      // An answer the session did not take is not remembered.
      open = false;
      state.revision = 6;
      t.receiveCompat({ compat: 'needs', needs: [need(0)] }, session);
      expect(t.answeredAt('title', 0)).toEqual({ revision: 5, kind: 'reuse' });
    } finally { t.dispose(); }
  });

});
