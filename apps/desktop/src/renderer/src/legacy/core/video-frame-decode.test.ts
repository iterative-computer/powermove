// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { seekVideoFrame, VideoFrameDecodeError } from './video-frame-decode';

function video(state: Record<string, unknown> = {}) {
  return Object.assign(new EventTarget(), { readyState: 1, seeking: false, currentTime: 0,
    error: null, pause: vi.fn(), ...state }) as unknown as HTMLVideoElement;
}
afterEach(() => vi.useRealTimers());

it('accepts loadeddata at the initial time without waiting for a nonexistent seeked event', async () => {
  const v = video(), pending = seekVideoFrame(v, 0);
  Object.assign(v, { readyState: 2 }); v.dispatchEvent(new Event('loadeddata'));
  await pending;
});

it('waits for metadata before assigning a seek and for actual decoded pixels afterwards', async () => {
  const v = video({ readyState: 0 });
  const pending = seekVideoFrame(v, 1.5);
  expect(v.currentTime).toBe(0);
  Object.assign(v, { readyState: 1 }); v.dispatchEvent(new Event('loadedmetadata'));
  expect(v.currentTime).toBe(1.5);
  let resolved = false; void pending.then(() => { resolved = true; });
  v.dispatchEvent(new Event('seeked')); await Promise.resolve(); expect(resolved).toBe(false);
  Object.assign(v, { readyState: 2 }); v.dispatchEvent(new Event('canplay'));
  await pending;
});

it('removes every listener and timer after timeout so a late completion cannot interfere', async () => {
  vi.useFakeTimers(); const v = video(), add = vi.spyOn(v, 'addEventListener'), remove = vi.spyOn(v, 'removeEventListener');
  const pending = seekVideoFrame(v, 0), failed = expect(pending).rejects.toBeInstanceOf(VideoFrameDecodeError);
  await vi.advanceTimersByTimeAsync(10_000); await failed;
  for (const [type, listener] of add.mock.calls) expect(remove).toHaveBeenCalledWith(type, listener);
  Object.assign(v, { readyState: 2 }); v.dispatchEvent(new Event('loadeddata'));
  expect(vi.getTimerCount()).toBe(0);
  // A failed request leaves no stale completion subscribed to this decoder.
  await seekVideoFrame(v, 0);
});

it('cancels readiness waits immediately and removes resources before a decoder completes', async () => {
  vi.useFakeTimers(); const v = video(), controller = new AbortController();
  const pending = seekVideoFrame(v, 1, { signal: controller.signal });
  controller.abort(); await expect(pending).rejects.toHaveProperty('name', 'AbortError');
  expect(vi.getTimerCount()).toBe(0);
});

it('clamps stale asset timestamps inside the actual media duration', async () => {
  vi.useFakeTimers();
  const v = video({ readyState: 2, duration: 1 });
  let time = 0;
  Object.defineProperty(v, 'currentTime', { get: () => time, set: value => { time = Math.min(1, value); } });
  const pending = seekVideoFrame(v, 1.2, { timeoutMs: 20 });
  const result = expect(pending).resolves.toBeUndefined();
  await vi.advanceTimersByTimeAsync(20); await result;
  expect(v.currentTime).toBeLessThan(1);
});

it('accepts microsecond clock rounding without accepting a different video frame', async () => {
  vi.useFakeTimers();
  const v = video({ readyState: 2 });
  let time = 0;
  Object.defineProperty(v, 'currentTime', { get: () => time, set: value => { time = Math.floor(value * 1e6) / 1e6; } });
  const pending = seekVideoFrame(v, 2 / 30, { tolerance: 1e-9, timeoutMs: 20 });
  const result = expect(pending).resolves.toBeUndefined();
  await vi.advanceTimersByTimeAsync(20); await result;
});

it('recognizes decoded readiness even when Chromium sends no further readiness event', async () => {
  vi.useFakeTimers();
  const v = video();
  const pending = seekVideoFrame(v, 1, { timeoutMs: 1000 });
  Object.assign(v, { readyState: 2 });
  const result = expect(pending).resolves.toBeUndefined();
  await vi.advanceTimersByTimeAsync(1000); await result;
});
