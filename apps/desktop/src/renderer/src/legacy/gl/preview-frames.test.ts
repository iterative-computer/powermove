import { expect, it, vi } from 'vitest';
import { PreviewFrames } from './preview-frames';

it('keeps recently revisited frames and releases older frames before exceeding the cap', () => {
  const dispose = vi.fn();
  const cache = new PreviewFrames<{ bytes: number; id: number }>(dispose);
  const frames = [0, 1, 2].map(id => ({ bytes: 16, id }));
  cache.put('0', frames[0]!, 32); cache.put('1', frames[1]!, 32);
  expect(cache.get('0')).toBe(frames[0]);
  cache.put('2', frames[2]!, 32);
  expect(cache.get('1')).toBeUndefined();
  expect(dispose).toHaveBeenCalledWith(frames[1]);
  expect(cache.bytes).toBe(32);
  cache.trim(16);
  expect(cache.get('2')).toBe(frames[2]);
  cache.clear(); cache.clear();
  expect(cache.bytes).toBe(0);
  expect(cache.count).toBe(0);
  expect(dispose).toHaveBeenCalledTimes(3);
});

it('declines frames larger than the cap and allocates nothing when disabled', () => {
  const dispose = vi.fn();
  const cache = new PreviewFrames(dispose);
  expect(cache.put('large', { bytes: 33 }, 32)).toBe(false);
  expect(cache.put('off', { bytes: 16 }, 0)).toBe(false);
  expect(cache.bytes).toBe(0);
  expect(dispose).not.toHaveBeenCalled();
});

it('makes room for a new scene without disposing the picture still displayed', () => {
  const dispose = vi.fn(), cache = new PreviewFrames<{ bytes: number; id: string }>(dispose);
  const visible = { bytes: 16, id: 'visible' }, older = { bytes: 16, id: 'older' };
  cache.put('visible', visible, 32); cache.put('older', older, 32);
  cache.trim(0, new Set([visible]));
  expect(cache.get('visible')).toBe(visible); expect(cache.bytes).toBe(16);
  expect(dispose).toHaveBeenCalledExactlyOnceWith(older);
  cache.clear(); expect(cache.bytes).toBe(0);
});
