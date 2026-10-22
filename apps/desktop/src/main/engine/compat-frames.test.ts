import { describe, expect, it, vi } from 'vitest';

import { OTHER_DOCUMENT_OPEN, type EngineCompatAsk } from '../../shared/engine-ipc';
import type { CompatFrameRequest } from '../../shared/engine-protocol';
import { ASK_TIMEOUT_MS, CompatFrames, INLINE_PIXEL_BYTES, validCompatAnswer } from './compat-frames';

const request: CompatFrameRequest = { document: 'doc-1', composition: 'c', layer: 'L7', time: 0, source: 'rasterOverride', scale: 1, revision: 4 };

function setup(windowIds: number[]) {
  const asks = new Map<number, EngineCompatAsk[]>();
  const timers: Array<{ fn: () => void; ms: number; cancelled: boolean }> = [];
  const publish = vi.fn(() => '/pm.0123456789abcdef01234567');
  const unlink = vi.fn();
  const frames = new CompatFrames({
    windows: () => windowIds.map((id) => ({ id, ask: (ask: EngineCompatAsk) => asks.set(id, [...(asks.get(id) ?? []), ask]) })),
    publishPixels: publish,
    unlinkPixels: unlink,
    schedule: (fn, ms) => {
      const t = { fn, ms, cancelled: false };
      timers.push(t);
      return { cancel: () => { t.cancelled = true; } };
    }
  });
  return { frames, asks, timers, publish, unlink };
}

const pixels = (ask: number, w = 2, h = 2) => ({ ask, status: 'pixels' as const, key: 'k', width: w, height: h, format: 'rgba8Premultiplied' as const, origin: [0, 0] as [number, number], scale: 1, data: new Uint8Array(w * h * 4).fill(9) });

describe('CompatFrames', () => {
  it('accepts compatibility answers at today’s raster density through 32', () => {
    for (const scale of [16.25, 24, 32, 32.01, 0, NaN, Infinity]) {
      expect(validCompatAnswer({ ...pixels(1), scale }), String(scale)).toBe(Number.isFinite(scale) && scale > 0 && scale <= 32);
    }
  });

  it('posts a burst of asks to each window as one message, and answers each', async () => {
    const singles: EngineCompatAsk[] = [];
    const batches: EngineCompatAsk[][] = [];
    const soon: Array<() => void> = [];
    const frames = new CompatFrames({
      windows: () => [{ id: 1, ask: (a: EngineCompatAsk) => singles.push(a), askMany: (asks: EngineCompatAsk[]) => batches.push(asks) }],
      publishPixels: null, unlinkPixels: null,
      soon: (fn) => soon.push(fn)
    });
    // An export frame's layers, asked back to back.
    const answers = Array.from({ length: 40 }, (_, i) => frames.serve({ ...request, layer: `L${i}` }));
    expect(batches).toEqual([]);
    expect(soon).toHaveLength(1);
    soon.shift()!();
    expect(singles).toEqual([]);
    expect(batches).toHaveLength(1);
    expect(batches[0]!.map((a) => a.request.layer)).toEqual(Array.from({ length: 40 }, (_, i) => `L${i}`));
    for (const ask of batches[0]!) frames.answer(1, { ask: ask.ask, status: 'native' });
    expect((await Promise.all(answers)).every((r) => (r as { native?: boolean }).native === true)).toBe(true);
    // A lone ask goes on its own.
    const one = frames.serve(request);
    soon.shift()!();
    expect(singles).toHaveLength(1);
    frames.answer(1, { ask: singles[0]!.ask, status: 'native' });
    await one;
  });

  it('sends the export snapshot\'s layer with the ask when main has it', async () => {
    const { frames, asks } = setup([1]);
    const answer = frames.serve(request, '{"id":"L7"}');
    const sent = asks.get(1)![0]!;
    expect(sent.layer).toBe('{"id":"L7"}');
    frames.answer(1, pixels(sent.ask));
    await answer;
    void frames.serve(request);
    expect('layer' in asks.get(1)![1]!).toBe(false);
  });

  it('hands a compatibility context\'s shared export frame to the windows inline, and answers its pass-through or its frame', async () => {
    const asks: EngineCompatAsk[] = [];
    const take = vi.fn(() => new Uint8Array(16).fill(5));
    const frames = new CompatFrames({ windows: () => [{ id: 1, ask: (a: EngineCompatAsk) => asks.push(a) }], publishPixels: () => '/pm.0123456789abcdef01234567', unlinkPixels: vi.fn(), takePixels: take });
    const context: CompatFrameRequest = { ...request, layer: 'c', source: 'compatContext', frame: { width: 2, height: 2, payload: { kind: 'sharedMemory', name: '/pm.aaaaaaaaaaaaaaaaaaaaaaaa', length: 16 } } };
    const first = frames.serve(context);
    expect(take).toHaveBeenCalledWith('/pm.aaaaaaaaaaaaaaaaaaaaaaaa', 16);
    expect(asks[0]!.request.frame).toEqual({ width: 2, height: 2, payload: { kind: 'inline', bytes: new Uint8Array(16).fill(5) } });
    frames.answer(1, { ask: asks[0]!.ask, status: 'native' });
    expect(await first).toMatchObject({ type: 'compatFrame', native: true });
    const second = frames.serve(context);
    frames.answer(1, pixels(asks[1]!.ask));
    expect(await second).toMatchObject({ type: 'compatFrame', width: 2, height: 2, payload: { kind: 'inline' } });
    // Without the addon's reader the frame cannot reach a window.
    const none = new CompatFrames({ windows: () => [{ id: 1, ask: () => {} }], publishPixels: null, unlinkPixels: null });
    await expect(none.serve(context)).rejects.toThrow(/shared pixels/);
  });

  it('asks every window and takes the first pixels', async () => {
    const { frames, asks } = setup([1, 2]);
    const answer = frames.serve(request);
    const ask = asks.get(1)![0]!.ask;
    expect(asks.get(2)![0]!.ask).toBe(ask);
    expect(frames.answer(1, { ask, status: 'unavailable', message: 'no such document' })).toBe(true);
    expect(frames.answer(2, pixels(ask))).toBe(true);
    const response = await answer;
    expect(response).toMatchObject({ type: 'compatFrame', key: 'k', width: 2, height: 2, payload: { kind: 'inline' } });
    // A second answer for the same ask is refused.
    expect(frames.answer(2, pixels(ask))).toBe(false);
  });

  it('fails with the reason when no window has the layer', async () => {
    const { frames, asks } = setup([1]);
    const answer = frames.serve(request);
    frames.answer(1, { ask: asks.get(1)![0]!.ask, status: 'unavailable', message: 'The override is not registered.' });
    await expect(answer).rejects.toMatchObject({ code: 'notHeadlessRenderable', message: 'The override is not registered.' });
    await expect(setup([]).frames.serve(request)).rejects.toMatchObject({ code: 'notHeadlessRenderable' });
  });

  it('names the window SVG images need when none has the project open', async () => {
    const svg: CompatFrameRequest = { ...request, source: 'svgImage', svg: { key: { content: 'ab'.repeat(32), width: 2, height: 2 }, asset: 'a1' } };
    await expect(setup([]).frames.serve(svg)).rejects.toMatchObject({
      code: 'notHeadlessRenderable',
      message: 'SVG images are drawn by the editor window that has this project open; open the project in a window to export it.'
    });
    // A window that has it answers with the bucket's pixels.
    const { frames, asks } = setup([3]);
    const answer = frames.serve(svg);
    expect(asks.get(3)![0]!.request.svg?.key.width).toBe(2);
    frames.answer(3, { ...pixels(asks.get(3)![0]!.ask), key: `${'ab'.repeat(32)}:2x2` });
    expect(await answer).toMatchObject({ type: 'compatFrame', key: `${'ab'.repeat(32)}:2x2`, width: 2, height: 2 });
    // Windows open on other documents: still the named message.
    const other = setup([4, 5]);
    const refused = other.frames.serve(svg);
    for (const id of [4, 5]) other.frames.answer(id, { ask: other.asks.get(id)![0]!.ask, status: 'unavailable', message: OTHER_DOCUMENT_OPEN });
    await expect(refused).rejects.toMatchObject({
      code: 'notHeadlessRenderable',
      message: 'SVG images are drawn by the editor window that has this project open; open the project in a window to export it.'
    });
    // A window that has the project and fails gives its own reason.
    const failing = setup([6, 7]);
    const failed = failing.frames.serve(svg);
    failing.frames.answer(6, { ask: failing.asks.get(6)![0]!.ask, status: 'unavailable', message: 'The SVG could not be decoded.' });
    failing.frames.answer(7, { ask: failing.asks.get(7)![0]!.ask, status: 'unavailable', message: OTHER_DOCUMENT_OPEN });
    await expect(failed).rejects.toMatchObject({ message: 'The SVG could not be decoded.' });
  });

  it('only the asked windows answer, and a closed window stops being waited for', async () => {
    const { frames, asks } = setup([1, 2]);
    const answer = frames.serve(request);
    const ask = asks.get(1)![0]!.ask;
    expect(frames.answer(3, pixels(ask))).toBe(false);
    frames.windowGone(1);
    frames.windowGone(2);
    await expect(answer).rejects.toMatchObject({ code: 'notHeadlessRenderable' });
  });

  it('times out, and shares large pixels through a segment it unlinks if untaken', async () => {
    const { frames, asks, timers, publish, unlink } = setup([1]);
    const slow = frames.serve(request);
    timers.find((t) => t.ms === ASK_TIMEOUT_MS)!.fn();
    await expect(slow).rejects.toMatchObject({ code: 'notHeadlessRenderable' });
    const big = frames.serve(request);
    const side = Math.ceil(Math.sqrt(INLINE_PIXEL_BYTES / 4)) + 1;
    frames.answer(1, pixels(asks.get(1)![1]!.ask, side, side));
    expect(await big).toMatchObject({ payload: { kind: 'sharedMemory', name: '/pm.0123456789abcdef01234567', length: side * side * 4 } });
    expect(publish).toHaveBeenCalledOnce();
    timers.at(-1)!.fn();
    expect(unlink).toHaveBeenCalledWith('/pm.0123456789abcdef01234567');
  });

  it('validates answers', () => {
    expect(validCompatAnswer(pixels(1))).toBe(true);
    expect(validCompatAnswer({ ...pixels(1), data: new Uint8Array(3) })).toBe(false);
    expect(validCompatAnswer({ ...pixels(1), width: 0 })).toBe(false);
    expect(validCompatAnswer({ ...pixels(1), format: 'rgb' })).toBe(false);
    expect(validCompatAnswer({ ...pixels(1), origin: [0, Number.NaN] })).toBe(false);
    expect(validCompatAnswer({ ask: 1, status: 'unavailable', message: 'x' })).toBe(true);
  });

  it('answers a pass-through as a native frame and carries the quad of a raster', async () => {
    const { frames, asks } = setup([1]);
    const native = frames.serve(request);
    expect(frames.answer(1, { ask: asks.get(1)![0]!.ask, status: 'native' })).toBe(true);
    expect(await native).toMatchObject({ type: 'compatFrame', native: true, payload: { kind: 'inline', bytes: new Uint8Array(0) } });
    const raster = frames.serve(request);
    expect(frames.answer(1, { ...pixels(asks.get(1)![1]!.ask), size: [40, 20], uv: [0, 0, 1, 1] })).toBe(true);
    expect(await raster).toMatchObject({ size: [40, 20], uv: [0, 0, 1, 1] });
    expect(validCompatAnswer({ ...pixels(9), size: [0, 1] })).toBe(false);
    expect(validCompatAnswer({ ...pixels(9), uv: [0, 0, 1] })).toBe(false);
  });
});
