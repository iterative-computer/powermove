// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createCompThumbnails, type CompThumbnails } from './comp-thumbnails.svelte';

const prepare = vi.hoisted(() => vi.fn());
vi.mock('./thumbnail-video-frames', () => ({ prepareThumbnailVideoFrames: prepare }));
let cache: CompThumbnails | undefined;
beforeEach(() => {
  vi.useFakeTimers(); prepare.mockReset();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ putImageData: vi.fn() } as any);
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/jpeg;poster');
  vi.stubGlobal('ImageData', class { constructor(readonly data: Uint8ClampedArray, readonly width: number, readonly height: number) {} });
});
afterEach(() => { cache?.dispose(); cache = undefined; vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
function fixture() {
  const oldFrames = new Map(), oldVersion = 4;
  const PM: any = { proj: { compId: 'root', w: 16, h: 8, dur: 4, layers: [] }, quality: .5,
    preparedVideoFrames: oldFrames, preparedVideoVersion: oldVersion,
    Comps: { list: () => [{ id: 'root' }] }, GL: { renderToPixels: vi.fn(() => new Uint8Array(16 * 8 * 4)) } };
  cache = createCompThumbnails(PM);
  let finish!: (value: any) => void;
  let reject!: (reason: unknown) => void;
  const pending = new Promise((resolve, fail) => { finish = resolve; reject = fail; });
  prepare.mockImplementation((_PM, _comp, _time, signal) => { signal(); return pending; });
  const prepared = { frames: new Map([['clip@2', document.createElement('canvas')]]), version: 'thumbnail:test', dispose: vi.fn() };
  const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
  return { PM, oldFrames, oldVersion, prepared, finish, reject, flush, get signal(): AbortSignal { return prepare.mock.calls[0]![3](); } };
}

it('owns prepared frames only during synchronous rendering and restores export ownership on success', async () => {
  const f = fixture();
  f.PM.GL.renderToPixels.mockImplementation(() => {
    expect(f.PM.preparedVideoFrames).toBe(f.prepared.frames); expect(f.PM.preparedVideoVersion).toBe('thumbnail:test');
    expect(f.PM.quality).toBe(1); return new Uint8Array(16 * 8 * 4);
  });
  cache!.refresh('one'); vi.advanceTimersByTime(400);
  expect(f.PM.preparedVideoFrames).toBe(f.oldFrames);
  expect(f.PM.GL.renderToPixels).not.toHaveBeenCalled();
  f.finish(f.prepared); await f.flush();
  expect(cache!.urls.root).toBe('data:image/jpeg;poster');
  expect(f.PM.preparedVideoFrames).toBe(f.oldFrames); expect(f.PM.preparedVideoVersion).toBe(f.oldVersion);
  expect(f.PM.quality).toBe(.5); expect(f.prepared.dispose).toHaveBeenCalledOnce();
});

it('restores frame/version ownership and releases copies when rendering throws', async () => {
  const f = fixture(); vi.spyOn(console, 'warn').mockImplementation(() => {});
  f.PM.GL.renderToPixels.mockImplementation(() => { throw new Error('GPU lost'); });
  cache!.refresh('one'); vi.advanceTimersByTime(400); f.finish(f.prepared); await f.flush();
  expect(f.PM.preparedVideoFrames).toBe(f.oldFrames); expect(f.PM.preparedVideoVersion).toBe(f.oldVersion);
  expect(f.PM.quality).toBe(.5); expect(f.prepared.dispose).toHaveBeenCalledOnce();
  expect(cache!.urls.root).toBeUndefined();
});

it.each(['refresh', 'project', 'dispose', 'playback', 'export'])('discards pending pixels after %s without modifying the viewer or export', async event => {
  const f = fixture();
  cache!.refresh('one'); vi.advanceTimersByTime(400); const signal = f.signal;
  if (event === 'refresh') cache!.refresh('two');
  if (event === 'project') f.PM.proj = { ...f.PM.proj };
  if (event === 'dispose') cache!.dispose();
  if (event === 'playback') f.PM.playing = true;
  if (event === 'export') f.PM.Export = { busy: true };
  if (event === 'playback' || event === 'export') vi.advanceTimersByTime(50);
  if (event !== 'project') expect(signal.aborted).toBe(true);
  f.finish(f.prepared); await f.flush();
  expect(f.PM.GL.renderToPixels).not.toHaveBeenCalled(); expect(f.prepared.dispose).toHaveBeenCalledOnce();
  expect(f.PM.preparedVideoFrames).toBe(f.oldFrames); expect(f.PM.preparedVideoVersion).toBe(f.oldVersion);
  expect(cache!.urls.root).toBeUndefined();
});

it('keeps routine cancellation quiet and retries once the interaction finishes', async () => {
  const f = fixture(), warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
  cache!.refresh('one'); vi.advanceTimersByTime(400);
  const signal = f.signal;
  f.PM.playing = true; vi.advanceTimersByTime(50); expect(signal.aborted).toBe(true);
  f.PM.playing = false;
  f.reject(Object.assign(new Error('cancelled'), { name: 'AbortError' })); await f.flush();
  expect(warn).not.toHaveBeenCalled();
  prepare.mockImplementation((_PM, _comp, _time, getSignal) => { expect(getSignal().aborted).toBe(false); return Promise.resolve(f.prepared); });
  vi.advanceTimersByTime(500); await f.flush();
  expect(cache!.urls.root).toBe('data:image/jpeg;poster'); expect(prepare).toHaveBeenCalledTimes(2);
});

it.each([false, true])('isolates an empty preparation and restores existing export ownership when rendering throws=%s', throws => {
  const f = fixture(); prepare.mockReturnValue(null);
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  f.PM.GL.renderToPixels.mockImplementation((_time: number, _w: number, _h: number, options: any) => {
    expect(options.isolatedVideoFrames).toBe(true);
    expect(f.PM.preparedVideoFrames).not.toBe(f.oldFrames); expect(f.PM.preparedVideoFrames.size).toBe(0);
    expect(f.PM.preparedVideoVersion).toBe('thumbnail:empty');
    if (throws) throw new Error('GPU unavailable');
    return new Uint8Array(16 * 8 * 4);
  });
  cache!.refresh('one'); vi.runAllTimers();
  expect(f.PM.preparedVideoFrames).toBe(f.oldFrames); expect(f.PM.preparedVideoVersion).toBe(f.oldVersion);
  expect(f.PM.quality).toBe(.5);
});
