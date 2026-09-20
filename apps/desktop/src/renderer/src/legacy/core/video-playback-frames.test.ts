import { afterEach, expect, it, vi } from 'vitest';
import { playbackVideoFrameAt, startPlaybackVideoFrames, stopPlaybackVideoFrames } from './video-playback-frames';

function decoder(width = 64, height = 64) {
  let next = 0;
  const callbacks = new Map<number, VideoFrameRequestCallback>();
  const video = Object.assign(new EventTarget(), {
    seeking: false, readyState: 2, videoWidth: width, videoHeight: height, pixel: 0, timestamp: 0,
    requestVideoFrameCallback: (callback: VideoFrameRequestCallback) => { callbacks.set(++next, callback); return next; },
    cancelVideoFrameCallback: (id: number) => callbacks.delete(id),
  }) as unknown as HTMLVideoElement & { pixel: number; timestamp: number };
  const resources: any[] = [];
  vi.stubGlobal('VideoFrame', class {
    pixel: number; timestamp: number; duration = 1e6 / 30;
    displayWidth = width; displayHeight = height; closed = false;
    constructor(source: typeof video) { this.pixel = source.pixel; this.timestamp = source.timestamp; resources.push(this); }
    close() { this.closed = true; }
  });
  let count = 0;
  return { video, resources, callbacks, present(time: number, pixel: number, callbackTime = time) {
    video.pixel = pixel; video.timestamp = time * 1e6;
    const [id, callback] = callbacks.entries().next().value!; callbacks.delete(id);
    callback(time * 1000, { mediaTime: callbackTime, presentedFrames: ++count } as VideoFrameCallbackMetadata);
  } };
}
afterEach(() => vi.unstubAllGlobals());

it('keeps pixels and their actual decoder timestamp together even when a callback is late', () => {
  const { video, present } = decoder();
  startPlaybackVideoFrames(video, 30, vi.fn());
  present(1, 10); present(1 + 1 / 30, 20, 1); present(1 + 2 / 30, 30);
  expect((playbackVideoFrameAt(video, 1.01)?.source as any).pixel).toBe(10);
  expect((playbackVideoFrameAt(video, 1.04)?.source as any).pixel).toBe(20);
  expect(playbackVideoFrameAt(video, .99)).toBeUndefined();
  expect(playbackVideoFrameAt(video, 1.11)).toBeUndefined();
  stopPlaybackVideoFrames(video);
});

it('does not stretch a frame over missing callbacks', () => {
  const { video, present } = decoder();
  startPlaybackVideoFrames(video, 30, vi.fn());
  present(0, 10); present(2 / 30, 30);
  expect(playbackVideoFrameAt(video, 1 / 30)).toBeUndefined();
  expect((playbackVideoFrameAt(video, 2 / 30)?.source as any).pixel).toBe(30);
  stopPlaybackVideoFrames(video);
});

it('bounds HD buffers, closes evicted frames and advances texture versions', () => {
  const { video, present, resources } = decoder(1920, 1080);
  startPlaybackVideoFrames(video, 30, vi.fn());
  present(0, 255);
  const first = playbackVideoFrameAt(video, 0)!;
  for (let i = 1; i <= 20; i++) present(i / 30, 0);
  expect(resources.filter(frame => !frame.closed)).toHaveLength(4);
  const last = playbackVideoFrameAt(video, 20 / 30)!;
  expect((last.source as any).pixel).toBe(0);
  expect(last.version).toBeGreaterThan(first.version);
  expect(playbackVideoFrameAt(video, 0)).toBeUndefined();
  stopPlaybackVideoFrames(video);
  expect(resources.every(frame => frame.closed)).toBe(true);
});

it('clears old-loop frames on seek and cancels callbacks on release', () => {
  const { video, present, callbacks, resources } = decoder();
  const ready = vi.fn();
  startPlaybackVideoFrames(video, 30, ready);
  startPlaybackVideoFrames(video, 30, ready);
  expect(callbacks.size).toBe(1);
  present(1, 10);
  video.dispatchEvent(new Event('seeking'));
  expect(resources[0].closed).toBe(true);
  expect(playbackVideoFrameAt(video, 1)).toBeUndefined();
  present(0, 20);
  expect((playbackVideoFrameAt(video, 0)?.source as any).pixel).toBe(20);
  stopPlaybackVideoFrames(video);
  expect(callbacks.size).toBe(0);
});
