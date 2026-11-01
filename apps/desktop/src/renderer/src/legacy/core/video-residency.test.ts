import { expect, it, vi } from 'vitest';
import { forgetVideo, isVideoParked, onVideoParked, parkIdleVideos, trackVideo, useVideo, videoResidency, videoSource } from './video-residency';

function video(src: string) {
  const attributes = new Map<string, string>([['src', src]]);
  const el = Object.assign(new EventTarget(), {
    paused: true, seeking: false, preload: 'metadata' as HTMLVideoElement['preload'], currentTime: 0,
    getAttribute: (name: string) => attributes.get(name) ?? null,
    removeAttribute: (name: string) => { attributes.delete(name); },
    pause: vi.fn(), load: vi.fn(),
  });
  Object.defineProperty(el, 'src', { get: () => attributes.get('src') ?? '', set: (value: string) => { attributes.set('src', value); } });
  return el as unknown as HTMLVideoElement;
}

it('releases idle decoders beyond the warm set and restores them on use', () => {
  const elements = Array.from({ length: 6 }, (_, index) => video(`app://powermove/_media/${index}`));
  elements.forEach((el, index) => trackVideo(el, index * 100));
  elements[2]!.currentTime = 3.5;
  // Two warm decoders survive; the rest have been idle long enough to release.
  expect(parkIdleVideos(new Set(), 10_000, { idleMs: 1000, warm: 2, maxIdleMs: 60_000 })).toBe(4);
  expect(elements.map(isVideoParked)).toEqual([true, true, true, true, false, false]);
  expect(elements[2]!.getAttribute('src')).toBeNull();
  expect(elements[2]!.load).toHaveBeenCalled();
  expect(videoSource(elements[2]!)).toBe('app://powermove/_media/2');

  useVideo(elements[2]!, 11_000);
  expect(isVideoParked(elements[2]!)).toBe(false);
  expect(elements[2]!.getAttribute('src')).toBe('app://powermove/_media/2');
  expect(elements[2]!.currentTime).toBe(3.5);
  expect(elements[2]!.preload).toBe('metadata');
  elements.forEach(forgetVideo);
});

it('never releases protected, recently used, playing or seeking elements', () => {
  const kept = video('a'), recent = video('b'), playing = video('c'), seeking = video('d'), idle = video('e');
  for (const el of [kept, playing, seeking, idle]) trackVideo(el, 0);
  trackVideo(recent, 9_500);
  (playing as any).paused = false; (seeking as any).seeking = true;
  expect(parkIdleVideos(new Set([kept]), 10_000, { idleMs: 1000, warm: 0 })).toBe(1);
  expect([kept, recent, playing, seeking, idle].map(isVideoParked)).toEqual([false, false, false, false, true]);
  // The warm set also expires.
  const warm = video('f'); trackVideo(warm, 0);
  expect(parkIdleVideos(new Set(), 100_000, { idleMs: 1000, warm: 8, maxIdleMs: 50_000 })).toBeGreaterThanOrEqual(1);
  expect(isVideoParked(warm)).toBe(true);
  for (const el of [kept, recent, playing, seeking, idle, warm]) forgetVideo(el);
});

it('forgets elements their owners unloaded', () => {
  const el = video('x'); trackVideo(el, 0);
  const before = videoResidency().tracked;
  el.removeAttribute('src');
  parkIdleVideos(new Set(), 10_000, { idleMs: 1, warm: 0 });
  expect(videoResidency().tracked).toBe(before - 1);
  expect(isVideoParked(el)).toBe(false);
});

it('caps resident decoders during a fast scrub and tells owners what was released', () => {
  const released: HTMLVideoElement[] = [];
  const stop = onVideoParked(el => released.push(el));
  const elements = Array.from({ length: 20 }, (_, index) => video(`app://powermove/_media/cap-${index}`));
  // All used within the idle window, the newest 610 ms ago.
  elements.forEach((el, index) => trackVideo(el, 9_000 + index * 10));
  const count = parkIdleVideos(new Set(), 9_800, { idleMs: 4000, warm: 8, cap: 12 });
  expect(count).toBe(8);
  // The oldest eight went; the twelve most recent stay.
  expect(elements.map(isVideoParked)).toEqual([...Array(8).fill(true), ...Array(12).fill(false)]);
  expect(released).toEqual(elements.slice(0, 8));
  stop();
  elements.forEach(forgetVideo);
});
