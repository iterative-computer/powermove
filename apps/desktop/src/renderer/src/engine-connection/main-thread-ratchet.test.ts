import { describe, expect, it } from 'vitest';
import { compareAndLowerMainThread, type PerfBaselines } from './main-thread-ratchet';

const name = 'desktop/playback';
const baseline = (): PerfBaselines => ({
  about: 'kept during a desktop update',
  benches: {
    [name]: {
      runner: 'desktop-vitest', units: 48, unit: 'frames',
      thread: { 'main_thread.posted': 47, 'main_thread.received': 56 }, all_threads: {}
    }
  }
});

describe('desktop perfbench ratchet', () => {
  it('fails on a rise and lowers only improved exact counts during update', () => {
    const file = baseline();
    const counts = { 'main_thread.posted': 48, 'main_thread.received': 50 };
    const checked = compareAndLowerMainThread(file, name, 48, counts, false);
    expect(checked.regressions).toEqual([expect.stringContaining('main_thread.posted: 48 > baseline 47')]);
    expect(checked.next).toBe(file);
    const updated = compareAndLowerMainThread(file, name, 48, counts, true);
    expect(updated.regressions).toHaveLength(1);
    expect(updated.next.benches[name]!.thread).toEqual({ 'main_thread.posted': 47, 'main_thread.received': 50 });
    expect(updated.next.about).toBe(file.about);
    expect(file.benches[name]!.thread['main_thread.received']).toBe(56);
  });

  it('never changes units or accepts a removed metric silently', () => {
    const file = baseline();
    const fewerFrames = compareAndLowerMainThread(file, name, 47, { 'main_thread.posted': 1 }, true);
    expect(fewerFrames.regressions).toEqual([expect.stringContaining('measured 47 frames')]);
    expect(fewerFrames.next).toBe(file);
    const missing = compareAndLowerMainThread(file, name, 48, { 'main_thread.posted': 1 }, true);
    expect(missing.regressions).toEqual([expect.stringContaining('main_thread.received: measured counter is missing')]);
    expect(missing.next.benches[name]!.thread['main_thread.received']).toBe(56);
  });

  it('treats a new metric on a known bench as a rise from zero', () => {
    const result = compareAndLowerMainThread(baseline(), name, 48, {
      'main_thread.posted': 47, 'main_thread.received': 56, 'main_thread.new': 1
    }, true);
    expect(result.regressions).toEqual([expect.stringContaining('main_thread.new: 1 > baseline 0')]);
    expect(result.next.benches[name]!.thread['main_thread.new']).toBeUndefined();
  });

  it('records a new bench only in update mode', () => {
    const file = baseline();
    const counts = { 'main_thread.posted': 4 };
    expect(compareAndLowerMainThread(file, 'desktop/new', 4, counts, false).regressions).toHaveLength(1);
    const updated = compareAndLowerMainThread(file, 'desktop/new', 4, counts, true);
    expect(updated.regressions).toEqual([]);
    expect(updated.next.benches['desktop/new']).toEqual({
      runner: 'desktop-vitest', units: 4, unit: 'frames', thread: counts, all_threads: {}
    });
    expect(updated.next.benches[name]).toBe(file.benches[name]);
  });
});
