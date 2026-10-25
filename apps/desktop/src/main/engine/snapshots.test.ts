import { describe, expect, it, vi } from 'vitest';

import type { EngineSnapshotRequest } from '../../shared/engine-ipc';
import type { Request, Response, SnapshotViewer } from '../../shared/engine-protocol';
import { RemoteError } from './protocol';
import { MAX_BLENDER_WAIT_MS, MAX_FALLBACK_BYTES, MAX_PER_WINDOW, RENDER_ALLOWANCE_MS, SnapshotService, answerBoundMs, fallbackBudgetBytes, validSnapshotRequest, type SnapshotEngine } from './snapshots';
import { FallbackInputs } from './fallback-inputs';
import { decode, encode } from './msgpack';

const request = (over: Partial<EngineSnapshotRequest['snapshot']> = {}, top: Partial<EngineSnapshotRequest> = {}): EngineSnapshotRequest => ({
  generation: 2,
  session: 5,
  revision: 41,
  snapshot: {
    composition: 'comp-1',
    time: 352_800_000,
    outputSize: { width: 4, height: 2 },
    target: { kind: 'frame', motionBlurSamples: 0, background: true, draft3d: false },
    pixels: 'opaque',
    class: 'sync',
    viewSeq: 3,
    waitMs: 1000,
    uploaded: [],
    ...over
  },
  ...top
});

const rendered = (payload: unknown, width = 4, height = 2): Response => ({
  type: 'snapshot',
  outcome: 'rendered',
  revision: 41,
  viewSeq: 3,
  width,
  height,
  pixels: 'opaque',
  payload,
  origin: [0, 0],
  quality: { resolution: 'full', representation: 'original', draft3d: false, cached: false, standIn: false },
  waitedUs: 10,
  renderUs: 900
}) as unknown as Response;

function engine(answer: (body: Request, revision: number | null) => Promise<Response>, ready = () => true): SnapshotEngine & { sent: Array<{ body: Request; revision: number | null }> } {
  const sent: Array<{ body: Request; revision: number | null }> = [];
  return {
    sent,
    isReady: ready,
    clientPort: (id) => (id === 9 ? { port: 33, generation: 2 } : null),
    request: (_process, body, _session, revision) => {
      sent.push({ body, revision: revision ?? null });
      return answer(body, revision ?? null);
    }
  };
}

describe('snapshot requests', () => {
  it('validates source identity, fit, color and source time', () => {
    const source = { kind: 'source' as const, asset: 'clip', fit: 'contain' as const, color: 'display' as const };
    expect(validSnapshotRequest(request({ target: source, time: 0 }))).toBe(true);
    expect(validSnapshotRequest(request({ target: { ...source, color: 'compositor' }, time: 0, class: 'async', pixels: 'straight' }))).toBe(true);
    expect(validSnapshotRequest(request({ target: { ...source, color: 'compositor' }, time: 0 }))).toBe(false);
    expect(validSnapshotRequest(request({ target: source, time: -1 }))).toBe(false);
    expect(validSnapshotRequest(request({ target: { ...source, asset: '' } }))).toBe(false);
    expect(validSnapshotRequest(request({ target: { ...source, fit: 'other' as never } }))).toBe(false);
    expect(validSnapshotRequest(request({ target: { ...source, color: 'other' as never } }))).toBe(false);
  });

  it('cancels only the originating window snapshot and forwards its signal', async () => {
    let pending!: AbortSignal;
    const e = engine(async () => { throw new Error('unused'); });
    e.request = async (_process, _body, _session, _revision, signal) => {
      pending = signal!;
      return new Promise((_resolve, reject) => signal!.addEventListener('abort', () => reject(new RemoteError({ code: 'cancelled', message: 'Cancelled.', scope: { kind: 'session' } } as never)), { once: true }));
    };
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    const result = service.snapshot(9, request({}, { requestId: 'one' }));
    service.cancel(10, 'one'); expect(pending.aborted).toBe(false);
    service.cancel(9, 'one');
    expect(await result).toMatchObject({ status: 'failed', code: 'cancelled' });
    expect(service.status().outstanding).toBe(0);
  });

  it('validates pinned viewer captures at the original backing size and rejects malformed cameras', () => {
    const viewer: SnapshotViewer = { viewSeq: 19, canvas: { width: 4, height: 2, crop: { x: 40, y: 20, width: 100, height: 50 }, css: { x: 5, y: 6, width: 200, height: 100 } },
      view: { camera: { view: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -2, -3, -4, 1], projection: { kind: 'orthographic', halfHeight: 3, near: -100, far: 100 } }, shading: 'solid', xray: true, localView: ['cube'] } };
    expect(validSnapshotRequest(request({ viewSeq: 19, viewer }))).toBe(true);
    expect(validSnapshotRequest(request({ viewSeq: 19, viewer: { ...viewer, view: null } }))).toBe(true);
    const bad = (change: (v: SnapshotViewer) => void) => {
      const pin = structuredClone(viewer); change(pin);
      expect(validSnapshotRequest(request({ viewSeq: 19, viewer: pin }))).toBe(false);
    };
    bad(v => { v.canvas.width = 5; });
    bad(v => { v.canvas.crop.width = 0; });
    bad(v => { v.canvas.css.x = Infinity; });
    bad(v => { v.viewSeq = -1; });
    expect(validSnapshotRequest(request({ viewSeq: 18, viewer }))).toBe(false);
    bad(v => { v.view!.camera!.view[0] = NaN; });
    bad(v => { v.view!.camera!.projection = { kind: 'perspective', fovYDegrees: 180, near: .1, far: 100 }; });
    expect(validSnapshotRequest(request({ viewSeq: 19, viewer, target: { kind: 'layer', layer: 'L1', scale: 1, crop: null } }))).toBe(false);
  });

  it('accepts any positive layer raster scale, as today’s PM.raster, independently of output scale', () => {
    for (const scale of [1e-9, 0.005, 16.25, 24, 32, 33, 50, 1000, Number.MAX_VALUE, 0, -1, NaN, Infinity, -Infinity]) {
      expect(validSnapshotRequest(request({ target: { kind: 'layer', layer: 'L1', scale, crop: null } })), String(scale)).toBe(Number.isFinite(scale) && scale > 0);
    }
    // The renderer sends scales above 2^53 as the largest safe integer
    // (`layerRasterScale`): every number above it is an integer the wire's
    // encoder refuses, which left such a request unanswered for 15 s.
    expect(decode(encode({ scale: Number.MAX_SAFE_INTEGER }))).toEqual({ scale: Number.MAX_SAFE_INTEGER });
    expect(() => encode({ scale: Number.MAX_VALUE })).toThrow(/safe range/);
  });

  it('budgets RGBA32F fallback pixels at the same pixel capacity with a 2× physical cap', () => {
    expect(fallbackBudgetBytes(MAX_FALLBACK_BYTES * 2, 32)).toBe(MAX_FALLBACK_BYTES);
    expect(fallbackBudgetBytes(MAX_FALLBACK_BYTES * 2 + 16, 32)).toBeGreaterThan(MAX_FALLBACK_BYTES);
    expect(fallbackBudgetBytes(MAX_FALLBACK_BYTES + 8, 16)).toBeGreaterThan(MAX_FALLBACK_BYTES);
  });
  it('accept the shapes the renderer sends and refuse the rest', () => {
    expect(validSnapshotRequest(request())).toBe(true);
    expect(validSnapshotRequest(request({ captionsHidden: true }))).toBe(true);
    expect(validSnapshotRequest(request({ captionsHidden: 'false' as never }))).toBe(false);
    expect(validSnapshotRequest(request({ target: { kind: 'layer', layer: 'L1', scale: 2, crop: { x: 0, y: 0, width: 8, height: 8 } } }))).toBe(true);
    expect(validSnapshotRequest(request({ outputSize: { width: 0, height: 2 } }))).toBe(false);
    expect(validSnapshotRequest(request({ outputSize: { width: 16_385, height: 2 } }))).toBe(false);
    expect(validSnapshotRequest(request({ waitMs: 0 }))).toBe(false);
    expect(validSnapshotRequest(request({ waitMs: 120_001 }))).toBe(false);
    expect(validSnapshotRequest(request({ class: 'urgent' as never }))).toBe(false);
    expect(validSnapshotRequest(request({ time: 0.5 }))).toBe(false);
    expect(validSnapshotRequest(request({ target: { kind: 'frame', motionBlurSamples: 257, background: true, draft3d: false } }))).toBe(false);
    expect(validSnapshotRequest(request({ target: { kind: 'layer', layer: 'L1', scale: 0, crop: null } }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: -1 }))).toBe(false);
    expect(validSnapshotRequest(request({ uploaded: ['a'.repeat(64)] }))).toBe(true);
    expect(validSnapshotRequest(request({ uploaded: [''] }))).toBe(false);
    expect(validSnapshotRequest(request({ uploaded: Array.from({ length: 65 }, () => 'k') }))).toBe(false);
    expect(validSnapshotRequest(null)).toBe(false);
  });

  it('bounds the exact compatibility needs a snapshot caller says it posted', () => {
    const need = { composition: 'comp-1', layer: 'title', time: 0, scale: 2, source: 'rasterOverride' as const, crop: { x: 256, y: 0, width: 512, height: 256 } };
    expect(validSnapshotRequest(request({ compatUploaded: [need] }))).toBe(true);
    expect(validSnapshotRequest(request({ compatUploaded: [] }))).toBe(true);
    for (const patch of [{ layer: '' }, { composition: '' }, { time: .5 }, { source: 'unknown' }, { scale: 0 }, { scale: 33 }, { crop: { x: 8192, y: 0, width: 1, height: 1 } }]) {
      expect(validSnapshotRequest(request({ compatUploaded: [{ ...need, ...patch } as never] }))).toBe(false);
    }
    expect(validSnapshotRequest(request({ compatUploaded: Array.from({ length: 257 }, () => need) }))).toBe(false);
  });

  it('accept a window\'s compatibility events only with its document', () => {
    const document = { id: 'doc', revision: 3 };
    const events = [{ type: 'compat', compat: 'producers', sources: ['rasterOverride'] }, { type: 'compat', compat: 'pixels', data: new Uint8Array(4) }];
    expect(validSnapshotRequest({ ...request({}, { session: null }), document, compatEvents: events })).toBe(true);
    expect(validSnapshotRequest({ ...request(), compatEvents: events })).toBe(false);
    expect(validSnapshotRequest({ ...request({}, { session: null }), document, compatEvents: [{ type: 'compat', compat: 'needs' }] })).toBe(false);
    expect(validSnapshotRequest({ ...request({}, { session: null }), document, compatEvents: [{ type: 'compat', compat: 'pixels', data: 'x' }] })).toBe(false);
    expect(validSnapshotRequest({ ...request({}, { session: null }), document, compatEvents: [{ type: 'shaderFallback' }] })).toBe(false);
  });

  it('accept a bounded Blender wait for async and idle captures only', () => {
    expect(validSnapshotRequest(request({ class: 'async', blenderWaitMs: 600_000 }))).toBe(true);
    expect(validSnapshotRequest(request({ class: 'idle', blenderWaitMs: 0 }))).toBe(true);
    expect(validSnapshotRequest(request({ blenderWaitMs: 0 }))).toBe(true);
    // A blocked thread never waits for Blender.
    expect(validSnapshotRequest(request({ blenderWaitMs: 1 }))).toBe(false);
    expect(validSnapshotRequest(request({ class: 'async', blenderWaitMs: MAX_BLENDER_WAIT_MS + 1 }))).toBe(false);
    expect(validSnapshotRequest(request({ class: 'async', blenderWaitMs: -1 }))).toBe(false);
    expect(validSnapshotRequest(request({ class: 'async', blenderWaitMs: 0.5 }))).toBe(false);
  });
});

describe('SnapshotService', () => {
  it("names the window's own port and revision and returns inline pixels", async () => {
    const pixels = new Uint8Array(32).fill(7);
    const e = engine(async () => rendered({ kind: 'inline', bytes: pixels }));
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    const reply = await service.snapshot(9, request());
    expect(reply).toMatchObject({ status: 'rendered', revision: 41, viewSeq: 3, width: 4, height: 2, pixels: 'opaque' });
    expect(reply.status === 'rendered' && reply.data).toEqual(pixels);
    expect(e.sent[0]).toMatchObject({ body: { type: 'snapshot', renderer: 33, session: 5 }, revision: 41 });
  });

  it('transfers native 4:2:2 source planes as a video surface without cloning them into renderer IPC', async () => {
    const video = { format: 'nv16', primaries: 'smpte170m', transfer: 'bt709', matrix: 'smpte170m', range: 'limited' };
    const planes = new Uint8Array(8).fill(128);
    const response = { ...rendered({ kind: 'inline', bytes: planes }, 2, 2), pixels: 'straight', video } as Response;
    const present = vi.fn(async () => ({ token: 'native-surface', width: 2, height: 2 }));
    const service = new SnapshotService({ engine: engine(async () => response), takePixels: null, sourceSurface: present, log: () => {} });
    const reply = await service.snapshot(9, request({ target: { kind: 'source', asset: 'clip', fit: 'contain', color: 'compositor' }, pixels: 'straight', class: 'async' }));
    expect(present).toHaveBeenCalledWith(9, 2, 2, video, planes, expect.any(Number));
    expect(reply).toMatchObject({ status: 'rendered', videoSurface: { token: 'native-surface', width: 2, height: 2 }, data: new Uint8Array() });
    const ordinary = await service.snapshot(9, request());
    expect(ordinary).toMatchObject({ status: 'failed', code: 'invalidMessage' });
    expect(present).toHaveBeenCalledOnce();
  });

  it('hands a shared coded frame to its surface without taking it into a main Buffer, and releases an unused one', async () => {
    const video = { format: 'p010le', primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' };
    const name = '/pm.0123456789abcdef01234567';
    const response = { ...rendered({ kind: 'sharedMemory', name, length: 24 }, 4, 2), pixels: 'straight', video } as Response;
    const take = vi.fn((_: string, length: number) => new Uint8Array(length));
    const present = vi.fn(async () => ({ token: 'segment-surface', width: 4, height: 2 }));
    const service = new SnapshotService({ engine: engine(async () => response), takePixels: take, sourceSurface: present, log: () => {} });
    const source = request({ target: { kind: 'source', asset: 'clip', fit: 'contain', color: 'compositor' }, pixels: 'straight', class: 'async' });
    expect(await service.snapshot(9, source)).toMatchObject({ status: 'rendered', videoSurface: { token: 'segment-surface' }, data: new Uint8Array() });
    expect(present).toHaveBeenCalledWith(9, 4, 2, video, { segment: name, length: 24 }, expect.any(Number));
    expect(take).not.toHaveBeenCalled();
    present.mockRejectedValueOnce(new Error('Source surface memory budget exceeded'));
    expect(await service.snapshot(9, source)).toMatchObject({ status: 'failed', code: 'internal' });
    expect(take).toHaveBeenCalledWith(name, 24);
  });

  it('uses distinct delivery stamps across raw and shared frames even at one clock tick', async () => {
    const clock = vi.spyOn(performance, 'now').mockReturnValue(42);
    try {
      let format = 'nv12';
      const present = vi.fn(async () => ({ token: 'surface', width: 2, height: 2 }));
      const e = engine(async () => ({ ...rendered({ kind: 'inline', bytes: new Uint8Array(format === 'nv12' ? 6 : 8) }, 2, 2),
        pixels: 'straight', video: { format, primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited' } }) as Response);
      const service = new SnapshotService({ engine: e, takePixels: null, sourceSurface: present, log: () => {} });
      const capture = () => service.snapshot(9, request({ target: { kind: 'source', asset: 'clip', fit: 'contain', color: 'compositor' }, pixels: 'straight', class: 'async' }));
      const first = await capture();
      format = 'nv16';
      await capture();
      format = 'nv12';
      const third = await capture();
      expect(first).toMatchObject({ status: 'rendered', videoPlanes: { timestamp: 42000 } });
      expect(present).toHaveBeenCalledWith(9, 2, 2, expect.anything(), expect.anything(), 42001);
      expect(third).toMatchObject({ status: 'rendered', videoPlanes: { timestamp: 42002 } });
    } finally { clock.mockRestore(); }
  });

  it('preserves native I420A planes for an alpha video frame without an RGB surface', async () => {
    const video = { format: 'i420a', primaries: 'smpte170m', transfer: 'bt709', matrix: 'smpte170m', range: 'limited' };
    const planes = new Uint8Array(10);
    const response = { ...rendered({ kind: 'inline', bytes: planes }, 2, 2), pixels: 'straight', video } as Response;
    const present = vi.fn(async () => { throw new Error('Alpha must not enter the opaque IOSurface path'); });
    const service = new SnapshotService({ engine: engine(async () => response), takePixels: null, sourceSurface: present, log: () => {} });
    const reply = await service.snapshot(9, request({ target: { kind: 'source', asset: 'alpha', fit: 'contain', color: 'compositor' }, pixels: 'straight', class: 'async' }));
    expect(reply).toMatchObject({ status: 'rendered', videoPlanes: { ...video, timestamp: expect.any(Number) }, data: planes });
    expect(present).not.toHaveBeenCalled();
  });

  it.each([
    [{ format: 'nv12', primaries: 'bt470bg', transfer: 'bt709', matrix: 'smpte170m', range: 'limited' }, 6],
    [{ format: 'rgba', primaries: 'bt709', transfer: 'srgb', matrix: 'rgb', range: 'full' }, 16]
  ])('sends %o frames without a GPU import', async (video, bytes) => {
    const planes = new Uint8Array(bytes);
    const response = { ...rendered({ kind: 'inline', bytes: planes }, 2, 2), pixels: 'straight', video } as Response;
    const present = vi.fn();
    const service = new SnapshotService({ engine: engine(async () => response), takePixels: null, sourceSurface: present, log: () => {} });
    const reply = await service.snapshot(9, request({ target: { kind: 'source', asset: 'clip', fit: 'contain', color: 'compositor' }, pixels: 'straight', class: 'async' }));
    expect(reply).toMatchObject({ status: 'rendered', videoPlanes: { ...video, timestamp: expect.any(Number) }, data: planes });
    expect(present).not.toHaveBeenCalled();
  });

  it('maps shared-memory results once through the bridge', async () => {
    const take = vi.fn(() => new Uint8Array(4 * 2 * 4).fill(3));
    const e = engine(async () => rendered({ kind: 'sharedMemory', name: '/pm.0123456789abcdef01234567', length: 32 }));
    const service = new SnapshotService({ engine: e, takePixels: take, log: () => {} });
    const reply = await service.snapshot(9, request());
    expect(reply.status).toBe('rendered');
    expect(take).toHaveBeenCalledWith('/pm.0123456789abcdef01234567', 32);
    expect(service.status().shared).toBe(1);
    // Without the addon it is a failure, never a crash.
    const blind = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    expect(await blind.snapshot(9, request())).toMatchObject({ status: 'failed', code: 'internal' });
  });

  it('refuses another window and a stale launch', async () => {
    const e = engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) }));
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    expect(await service.snapshot(4, request())).toMatchObject({ status: 'failed', code: 'unknownSession' });
    expect(await service.snapshot(9, request({}, { generation: 1 }))).toMatchObject({ status: 'failed', code: 'unknownSession' });
    expect(e.sent).toHaveLength(0);
  });

  it('passes needsRenderer through and engine errors as codes', async () => {
    let answer: Response | Error = { type: 'snapshot', outcome: 'needsRenderer', revision: 41, fallback: [], compat: [] } as unknown as Response;
    const e = engine(async () => {
      if (answer instanceof Error) throw answer;
      return answer;
    });
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    expect(await service.snapshot(9, request())).toEqual({ status: 'needsRenderer', revision: 41, fallback: [], compat: [], svg: [] });
    answer = { type: 'snapshot', outcome: 'empty', revision: 41 } as unknown as Response;
    expect(await service.snapshot(9, request())).toEqual({ status: 'empty', revision: 41 });
    answer = new RemoteError({ code: 'unsupported', scope: { kind: 'request', id: 1 }, message: 'The new engine cannot draw this frame: x.', retry: { kind: 'never' }, detail: null } as never);
    expect(await service.snapshot(9, request())).toMatchObject({ status: 'failed', code: 'unsupported', message: 'The new engine cannot draw this frame: x.' });
    answer = new Error('the interactive engine is not connected');
    expect(await service.snapshot(9, request())).toMatchObject({ status: 'failed', code: 'unavailable' });
  });

  it('hands an effect request its textures, which the engine sent ahead of the answer', async () => {
    const inputs = new FallbackInputs();
    const effect = { key: 'f'.repeat(64), layer: 'L1', vertex: 'v', fragment: 'f', width: 2, height: 1, calls: [], stage: 'bind', message: '', textures: 1, units: [], morePasses: [] };
    const layer = { ...effect, key: '0'.repeat(64), textures: 0 };
    const e = {
      ...engine(async () => {
        // Bands arrive on main's port before the answer they belong to.
        inputs.receive({ type: 'shaderFallback', fallback: 'input', key: effect.key, texture: 0, width: 2, height: 1, row: 0, rows: 1, rgba16f: new Uint8Array(16).fill(9) });
        return { type: 'snapshot', outcome: 'needsRenderer', revision: 41, fallback: [effect, layer], compat: [] } as unknown as Response;
      }),
      fallbackTextures: (key: string, count: number, width: number, height: number) => inputs.take(key, count, width, height)
    };
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    const reply = await service.snapshot(9, request());
    expect(reply).toMatchObject({ status: 'needsRenderer', fallback: [effect, layer], fallbackTextures: [{ key: effect.key, textures: [new Uint8Array(16).fill(9)] }] });
    expect(inputs.held).toBe(0);
  });

  it('bounds the wait and frees a late segment', async () => {
    vi.useFakeTimers();
    try {
      let late: (r: Response) => void = () => {};
      const e = engine(() => new Promise<Response>((resolve) => { late = resolve; }));
      const take = vi.fn(() => new Uint8Array(32));
      const service = new SnapshotService({ engine: e, takePixels: take, log: () => {}, now: () => Date.now() });
      const reply = service.snapshot(9, request({ waitMs: 200 }));
      await vi.advanceTimersByTimeAsync(200 + RENDER_ALLOWANCE_MS + 1);
      expect(await reply).toMatchObject({ status: 'failed', code: 'timeout' });
      late(rendered({ kind: 'sharedMemory', name: '/pm.0123456789abcdef01234567', length: 32 }));
      await vi.advanceTimersByTimeAsync(0);
      expect(take).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it('waits for Blender within its bound, and the request wait again after it', async () => {
    expect(answerBoundMs(request({ waitMs: 200 }).snapshot)).toBe(200 + RENDER_ALLOWANCE_MS);
    const blender = request({ class: 'async', waitMs: 200, blenderWaitMs: 60_000 });
    expect(answerBoundMs(blender.snapshot)).toBe(400 + 60_000 + RENDER_ALLOWANCE_MS);
    vi.useFakeTimers();
    try {
      let answer: (r: Response) => void = () => {};
      const e = engine(() => new Promise<Response>((resolve) => { answer = resolve; }));
      const service = new SnapshotService({ engine: e, takePixels: () => new Uint8Array(0), log: () => {}, now: () => Date.now() });
      const reply = service.snapshot(9, blender);
      // Past the plain bound: Blender may still be rendering.
      await vi.advanceTimersByTimeAsync(200 + RENDER_ALLOWANCE_MS + 1_000);
      answer(rendered({ kind: 'inline', bytes: new Uint8Array(32) }));
      expect(await reply).toMatchObject({ status: 'rendered' });
      expect((e.sent[0]!.body as { snapshot: { blenderWaitMs: number } }).snapshot.blenderWaitMs).toBe(60_000);
      // And it is bounded.
      const late = service.snapshot(9, blender);
      await vi.advanceTimersByTimeAsync(answerBoundMs(blender.snapshot) + 1);
      expect(await late).toMatchObject({ status: 'failed', code: 'timeout' });
    } finally {
      vi.useRealTimers();
    }
  });

  it('waits for a starting engine within the request wait, then fails', async () => {
    let ready = false;
    let t = 0;
    const e = engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) }), () => ready);
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {}, now: () => t, sleep: async (ms) => { t += ms; if (t >= 100) ready = true; } });
    expect((await service.snapshot(9, request({ waitMs: 1000 }))).status).toBe('rendered');
    ready = false;
    t = 0;
    const never = new SnapshotService({ engine: engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) }), () => false), takePixels: null, log: () => {}, now: () => t, sleep: async (ms) => { t += ms; } });
    expect(await never.snapshot(9, request({ waitMs: 300 }))).toMatchObject({ status: 'failed', code: 'unavailable' });
  });

  it('caps snapshots in flight per window and in bytes', async () => {
    const waiting: Array<() => void> = [];
    const e = engine(() => new Promise<Response>((resolve) => waiting.push(() => resolve(rendered({ kind: 'inline', bytes: new Uint8Array(32) })))));
    const service = new SnapshotService({ engine: e, takePixels: null, log: () => {} });
    const replies = Array.from({ length: MAX_PER_WINDOW }, () => service.snapshot(9, request()));
    expect(await service.snapshot(9, request())).toMatchObject({ status: 'failed', code: 'refused' });
    await Promise.resolve();
    for (const go of waiting) go();
    for (const reply of await Promise.all(replies)) expect(reply.status).toBe('rendered');
    expect(service.status()).toMatchObject({ outstanding: 0, outstandingBytes: 0 });
    // A result past the byte cap is refused before it reaches the engine.
    const huge = request({ outputSize: { width: 16_384, height: 16_384 } });
    expect(validSnapshotRequest(huge)).toBe(false);
  });

  it('never rejects on garbage', async () => {
    const service = new SnapshotService({ engine: engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) })), takePixels: null, log: () => {} });
    expect(await service.snapshot(9, { hello: 'world' })).toMatchObject({ status: 'failed', code: 'invalidMessage' });
  });
});

describe('sync snapshots served by main’s session for the window', () => {
  const doc = { id: 'doc', revision: 41, hash: 'ab'.repeat(32), json: new TextEncoder().encode('{}') };

  it('accepts an opaque native reference but rejects mixed native and JSON/hash bodies', () => {
    const reference = { id: 'doc', revision: 41, nativeKernel: true as const };
    expect(validSnapshotRequest(request({}, { session: null, document: reference }))).toBe(true);
    expect(validSnapshotRequest(request({}, { session: null, document: { ...reference, json: doc.json } }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: { ...reference, hash: doc.hash } }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: { ...reference, nativeKernel: false as never } }))).toBe(false);
  });

  it('accept a document in place of the session, never neither', () => {
    expect(validSnapshotRequest(request({}, { session: null, document: doc }))).toBe(true);
    expect(validSnapshotRequest(request({}, { session: null, document: { id: 'doc', revision: 41 } }))).toBe(true);
    expect(validSnapshotRequest(request({}, { session: null }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: { id: 'doc', revision: 41, json: new Uint8Array(2) } }))).toBe(false);
    expect(validSnapshotRequest(request({}, { fallbackPixels: [] }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, fallbackPixels: [{ key: 'k', width: 1, height: 1, rgba16f: new Uint8Array(8) }] }))).toBe(true);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, fallbackPixels: [{ key: 'k', width: 1, height: 1, rgba16f: new Uint8Array(4) }] }))).toBe(false);
    const floatBits = new Uint32Array([0x3f800008, 0x3eaaaaab, 0x80000000, 0x3f800000]);
    const full = { key: 'f', width: 1, height: 1, rgba16f: new Uint8Array(0), rgba32f: new Uint8Array(floatBits.buffer) };
    const detached = (fallbackPixels: unknown[]) => request({}, { session: null, document: doc, fallbackPixels: fallbackPixels as EngineSnapshotRequest['fallbackPixels'] });
    expect(validSnapshotRequest(detached([full]))).toBe(true);
    expect(validSnapshotRequest(detached([{ ...full, rgba32f: new Uint8Array(15) }]))).toBe(false);
    expect(validSnapshotRequest(detached([{ ...full, rgba16f: new Uint8Array(8) }]))).toBe(false);
    expect(validSnapshotRequest(detached([{ ...full, rgba32f: undefined }]))).toBe(false);
    const svgKey = { content: 'c'.repeat(64), width: 2, height: 1 };
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: svgKey, data: new Uint8Array(8) }] }))).toBe(true);
    expect(validSnapshotRequest(request({}, { svgPixels: [{ key: svgKey, data: new Uint8Array(8) }] }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: svgKey, data: new Uint8Array(4) }] }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: { ...svgKey, content: 'C'.repeat(64) }, data: new Uint8Array(8) }] }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: { ...svgKey, width: 9000 }, data: new Uint8Array(9000 * 4) }] }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: { ...svgKey, texture: true, width: 16384 }, data: new Uint8Array(16384 * 4) }] }))).toBe(true);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: { ...svgKey, texture: true, width: 16385 }, data: new Uint8Array(16385 * 4) }] }))).toBe(false);
    expect(validSnapshotRequest(request({}, { session: null, document: doc, svgPixels: [{ key: { ...svgKey, texture: false }, data: new Uint8Array(8) }] }))).toBe(false);
  });

  it('snapshot main’s session, the host naming itself, with no renderer session or launch match needed', async () => {
    const e = engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) }));
    const prepare = vi.fn(async () => ({ port: 3, session: 77 }));
    const service = new SnapshotService({ engine: e, takePixels: null, windowSessions: { prepare }, log: () => {} });
    const reply = await service.snapshot(12, request({}, { session: null, generation: 0, document: doc }));
    expect(reply.status).toBe('rendered');
    expect(prepare).toHaveBeenCalledWith(12, doc, [], [], []);
    expect(e.sent[0]!.body).toMatchObject({ type: 'snapshot', renderer: 3, session: 77 });
    expect(service.status().windowSessions).toBe(1);
  });

  it('derives a native reference owner from the IPC sender, never a renderer field', async () => {
    const e = engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) }));
    const prepare = vi.fn(async () => ({ port: 3, session: 77 }));
    const service = new SnapshotService({ engine: e, takePixels: null, windowSessions: { prepare }, log: () => {} });
    const reference = { id: 'doc', revision: 41, nativeKernel: true as const };
    expect(await service.snapshot(12, request({}, { session: null, generation: 0, document: reference }))).toMatchObject({ status: 'rendered' });
    expect(prepare).toHaveBeenCalledExactlyOnceWith(12, reference, [], [], []);
    expect(reference).not.toHaveProperty('owner');
    expect(e.sent[0]!.body).toMatchObject({ type: 'snapshot', renderer: 3, session: 77 });
  });

  it('answer needsDocument, and a bounded failure when main’s session is not ready in time', async () => {
    const e = engine(async () => rendered({ kind: 'inline', bytes: new Uint8Array(32) }));
    const needs = new SnapshotService({ engine: e, takePixels: null, windowSessions: { prepare: async () => 'needsDocument' as const }, log: () => {} });
    expect(await needs.snapshot(12, request({}, { session: null, document: { id: 'doc', revision: 41 } }))).toMatchObject({ status: 'failed', code: 'needsDocument' });
    vi.useFakeTimers();
    try {
      const stuck = new SnapshotService({ engine: e, takePixels: null, windowSessions: { prepare: () => new Promise(() => {}) }, log: () => {} });
      const pending = stuck.snapshot(12, request({ waitMs: 50 }, { session: null, document: doc }));
      await vi.advanceTimersByTimeAsync(50 + RENDER_ALLOWANCE_MS + 1);
      expect(await pending).toMatchObject({ status: 'failed', code: 'timeout' });
    } finally {
      vi.useRealTimers();
    }
  });
});

