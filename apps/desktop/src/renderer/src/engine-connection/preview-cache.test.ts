import { describe, expect, it, vi } from 'vitest';
import { installNativePreviewCache } from './preview-cache';

function fixture() {
  const PM: any = {
    proj: { id: 'P', fps: 24, dur: 1, work: [0, 1 / 12] }, time: .5,
    animVersion: () => 1, playing: false,
    pause: vi.fn(() => { PM.playing = false; }), play: vi.fn(() => { PM.playing = true; }),
    setTime: vi.fn((time) => { PM.time = time; }), invalidate: vi.fn(), toast: vi.fn(),
    Memory: { budget: () => 1024 * 1024 },
    bus: { on: vi.fn(() => () => {}), emit: vi.fn() },
  };
  const calls: Array<[number, boolean, boolean?]> = [];
  let reads = 0;
  const session: any = { previewSize: () => ({ width: 64, height: 64 }),
    previewCache: (...args: [number, boolean, boolean?]) => calls.push(args),
    cacheFrame: vi.fn(async () => ({ quality: { cached: reads++ >= 2 } })),
  };
  const dispose = installNativePreviewCache(PM, () => session);
  return { PM, session, calls, dispose };
}

describe('native Render Work Area', () => {
  it('warms exact frames, verifies their lease residency, and uses the ordinary transport', async () => {
    const f = fixture();
    try {
      await f.PM.Preview.cache();
      expect(f.session.cacheFrame.mock.calls).toEqual([[0], [1 / 24], [0]]);
      expect(f.PM.Preview.count).toBe(2);
      expect(f.PM.Preview.bytes).toBe(2 * 64 * 64 * 8);
      expect(f.PM.play).toHaveBeenCalledOnce();
      expect(f.PM.Preview.active).toBe(true);
      expect(f.calls.at(-1)).toEqual([1024 * 1024, false]);
    } finally { f.dispose(); }
  });
  it('releases native memory and restores time after decode failure', async () => {
    const f = fixture();
    f.session.cacheFrame.mockRejectedValueOnce(new Error('source went offline'));
    try {
      await f.PM.Preview.cache();
      expect(f.PM.play).not.toHaveBeenCalled();
      expect(f.PM.time).toBe(.5);
      expect(f.PM.Preview.count).toBe(0);
      expect(f.PM.Preview.preparing).toBe(false);
      expect(f.calls.at(-1)).toEqual([0, false, true]);
      expect(f.PM.toast).toHaveBeenCalledWith('source went offline', 6000);
    } finally { f.dispose(); }
  });
  it('does not claim a cached preview when the native lease evicted its first frame', async () => {
    const f = fixture();
    f.session.cacheFrame.mockResolvedValue({ quality: { cached: false } });
    try {
      await f.PM.Preview.cache();
      expect(f.PM.play).not.toHaveBeenCalled();
      expect(f.PM.toast.mock.calls[0]![0]).toMatch(/available memory/);
      expect(f.PM.Preview.bytes).toBe(0);
    } finally { f.dispose(); }
  });
  it('does not allocate or render when preview memory is off', async () => {
    const f = fixture(); f.PM.Memory.budget = () => 0;
    try {
      await f.PM.Preview.cache();
      expect(f.session.cacheFrame).not.toHaveBeenCalled();
      expect(f.PM.toast.mock.calls[0]![0]).toMatch(/memory is off/);
    } finally { f.dispose(); }
  });
});
