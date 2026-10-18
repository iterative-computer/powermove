import { describe, expect, it, vi } from 'vitest';

import type { EngineSnapshotRequest } from '../../shared/engine-ipc';
import type { Request, Response } from '../../shared/engine-protocol';
import { RemoteError } from './protocol';
import { MAX_PER_WINDOW, RENDER_ALLOWANCE_MS, SnapshotService, validSnapshotRequest, type SnapshotEngine } from './snapshots';

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
  it('accept the shapes the renderer sends and refuse the rest', () => {
    expect(validSnapshotRequest(request())).toBe(true);
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
    expect(validSnapshotRequest(null)).toBe(false);
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
    expect(await service.snapshot(9, request())).toEqual({ status: 'needsRenderer', revision: 41, fallback: [], compat: [] });
    answer = new RemoteError({ code: 'unsupported', scope: { kind: 'request', id: 1 }, message: 'The new engine cannot draw this frame: x.', retry: { kind: 'never' }, detail: null } as never);
    expect(await service.snapshot(9, request())).toMatchObject({ status: 'failed', code: 'unsupported', message: 'The new engine cannot draw this frame: x.' });
    answer = new Error('the interactive engine is not connected');
    expect(await service.snapshot(9, request())).toMatchObject({ status: 'failed', code: 'unavailable' });
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
