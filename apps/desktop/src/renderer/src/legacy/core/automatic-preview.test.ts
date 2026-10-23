import { describe, expect, it, vi } from 'vitest';
import { createAutomaticPreview, type AutomaticPreviewState } from './automatic-preview';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function harness(overrides: Partial<AutomaticPreviewState> = {}) {
  let clock = 0;
  const state: AutomaticPreviewState = {
    key: 'original', project: {}, time: 0, fps: 24, start: 0, end: 10,
    blocked: false, budget: 100, bytes: 0, frameBytes: 10, ...overrides,
  };
  const cached = new Set<number>();
  const timers: { at: number; callback: () => void; canceled: boolean; fired: boolean }[] = [];
  const schedule = vi.fn((callback: () => void, delay: number) => {
    const timer = { at: clock + delay, callback, canceled: false, fired: false };
    timers.push(timer);
    return () => { timer.canceled = true; };
  });
  const has = vi.fn((time: number) => cached.has(Math.round(time * state.fps)));
  const prepare = vi.fn(async (time: number, valid: () => boolean) => {
    if (!valid()) return false;
    cached.add(Math.round(time * state.fps));
    state.bytes += Math.ceil(state.frameBytes);
    return true;
  });
  const scheduler = createAutomaticPreview({ state: () => state, has, prepare, schedule, now: () => clock });
  const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  const advance = async (ms: number) => {
    const until = clock + ms;
    for (;;) {
      const next = timers.filter(timer => !timer.canceled && !timer.fired && timer.at <= until)
        .sort((a, b) => a.at - b.at)[0];
      if (!next) break;
      clock = next.at;
      next.fired = true;
      next.callback();
      await settle();
    }
    clock = until;
    await settle();
  };
  return { state, cached, timers, schedule, has, prepare, scheduler, advance, settle,
    jump: (ms: number) => { clock += ms; },
    queued: () => timers.filter(timer => !timer.canceled && !timer.fired),
  };
}

describe('automatic RAM preview', () => {
  it('waits for idle without replacing timers or scanning frames on repeated RAF requests', async () => {
    const h = harness();
    h.scheduler.request();
    for (let i = 0; i < 1000; i++) h.scheduler.request();
    expect(h.schedule).toHaveBeenCalledTimes(1);
    expect(h.has).not.toHaveBeenCalled();
    await h.advance(749);
    expect(h.prepare).not.toHaveBeenCalled();
    await h.advance(1);
    expect(h.prepare).toHaveBeenCalledTimes(1);
  });

  it('fills forward on exact 24 fps frames, flooring the playhead rather than rounding it', async () => {
    const h = harness({ time: 1.249, budget: 40 });
    h.scheduler.request();
    await h.advance(1000);
    expect(h.prepare.mock.calls.map(([time]) => time)).toEqual([29 / 24, 30 / 24, 31 / 24, 32 / 24]);
    expect(h.state.bytes).toBe(40);
    expect(h.queued()).toHaveLength(0);
  });

  it('stops at the RAM cap without evicting forward frames or spinning while full', async () => {
    const h = harness({ budget: 35, bytes: 10 });
    h.cached.add(0);
    h.scheduler.request();
    await h.advance(5000);
    expect(h.prepare.mock.calls.map(([time]) => time)).toEqual([1 / 24, 2 / 24]);
    expect([...h.cached]).toEqual([0, 1, 2]);
    expect(h.state.bytes).toBe(30);
    expect(h.queued()).toHaveLength(0);
    const scans = h.has.mock.calls.length;
    const schedules = h.schedule.mock.calls.length;
    for (let i = 0; i < 1000; i++) h.scheduler.request();
    await h.advance(60_000);
    expect(h.schedule).toHaveBeenCalledTimes(schedules);
    expect(h.has).toHaveBeenCalledTimes(scans);
    expect(h.prepare).toHaveBeenCalledTimes(2);
  });

  it('prepares a whole 73-second Quarter-resolution work area when it fits the RAM limit', async () => {
    const frameBytes = 480 * 270 * 4;
    const h = harness({ end: 73, frameBytes, budget: 1024 * 1024 * 1024 });
    h.scheduler.request();
    await h.advance(100_000);
    expect(h.prepare).toHaveBeenCalledTimes(73 * 24);
    expect(h.prepare.mock.calls.at(-1)![0]).toBe((73 * 24 - 1) / 24);
    expect(h.state.bytes).toBe(73 * 24 * frameBytes);
    expect(h.state.bytes).toBeLessThan(h.state.budget);
    expect(h.queued()).toHaveLength(0);
  });

  it('limits forward preparation by RAM capacity and stops at the work-area end', async () => {
    const h = harness({ start: 1, end: 1.25, time: 1, frameBytes: 10.1, budget: 55 });
    h.scheduler.request();
    await h.advance(5000);
    expect(h.prepare.mock.calls.map(([time]) => time)).toEqual([24 / 24, 25 / 24, 26 / 24, 27 / 24, 28 / 24]);
    expect(h.state.bytes).toBe(55);
    expect(h.queued()).toHaveLength(0);
    h.state.bytes = 0;
    h.cached.clear();
    h.state.budget = 1000;
    h.scheduler.request();
    await h.advance(5000);
    expect(h.prepare.mock.calls.slice(5).map(([time]) => time)).toEqual([24 / 24, 25 / 24, 26 / 24, 27 / 24, 28 / 24, 29 / 24]);
    expect(h.queued()).toHaveLength(0);
  });

  it('bounds frame bookkeeping at 4096 even when RAM and a long work area allow more', async () => {
    const h = harness({ end: 1000, frameBytes: 1, budget: 100_000 });
    h.scheduler.request();
    await h.advance(300_000);
    expect(h.prepare).toHaveBeenCalledTimes(4096);
    expect(h.prepare.mock.calls.at(-1)![0]).toBe(4095 / 24);
    expect(h.state.bytes).toBe(4096);
    expect(h.queued()).toHaveLength(0);
    const scans = h.has.mock.calls.length;
    for (let request = 0; request < 100; request++) h.scheduler.request();
    expect(h.has).toHaveBeenCalledTimes(scans);
    expect(h.queued()).toHaveLength(0);
  });

  it('keeps every prepared time inside fractional work-area boundaries and excludes the end', async () => {
    const h = harness({ start: 1.02, end: 1.5, time: 0, budget: 1000 });
    h.scheduler.request();
    await h.advance(5000);
    expect(h.prepare.mock.calls.map(([time]) => time)).toEqual(Array.from({ length: 11 }, (_, index) => (25 + index) / 24));
    expect(h.prepare.mock.calls.every(([time]) => time >= 1.02 && time < 1.5)).toBe(true);
  });

  it('does not prepare when the work range contains no project frame', async () => {
    const h = harness({ start: .01, end: .02 });
    h.scheduler.request();
    await h.advance(10_000);
    expect(h.prepare).not.toHaveBeenCalled();
    expect(h.schedule).not.toHaveBeenCalled();
  });

  it.each([
    { name: 'disabled cache', change: { budget: 0 } },
    { name: 'too little RAM for one rounded-up frame', change: { frameBytes: 10.1, budget: 10.9 } },
    { name: 'missing project', change: { project: null } },
    { name: 'invalid frame size', change: { frameBytes: NaN } },
    { name: 'invalid frame rate', change: { fps: 0 } },
    { name: 'invalid RAM accounting', change: { bytes: -1 } },
  ])('does not schedule work with $name', async ({ change }) => {
    const h = harness(change);
    h.scheduler.request();
    await h.advance(10_000);
    expect(h.prepare).not.toHaveBeenCalled();
    expect(h.schedule).not.toHaveBeenCalled();
  });

  it.each(['playing', 'exporting', 'editing', 'hidden editor'])('stops on %s and waits for a fresh idle period afterward', async () => {
    const h = harness();
    h.scheduler.request();
    await h.advance(700);
    h.state.blocked = true;
    h.scheduler.request();
    await h.advance(5000);
    expect(h.prepare).not.toHaveBeenCalled();
    expect(h.queued()).toHaveLength(0);
    h.state.blocked = false;
    h.scheduler.request();
    await h.advance(749);
    expect(h.prepare).not.toHaveBeenCalled();
    await h.advance(1);
    expect(h.prepare).toHaveBeenCalledTimes(1);
  });

  it.each([
    ['content edit', (s: AutomaticPreviewState) => { s.key = 'edited'; }],
    ['project switch', (s: AutomaticPreviewState) => { s.project = {}; }],
    ['scrub', (s: AutomaticPreviewState) => { s.time = 2; }],
    ['frame rate change', (s: AutomaticPreviewState) => { s.fps = 30; }],
    ['resolution change', (s: AutomaticPreviewState) => { s.frameBytes = 40; }],
    ['work area change', (s: AutomaticPreviewState) => { s.end = 4; }],
    ['RAM limit change', (s: AutomaticPreviewState) => { s.budget = 60; }],
    ['cache disabled', (s: AutomaticPreviewState) => { s.budget = 0; }],
  ])('invalidates a pending frame after %s, including before another RAF request', async (_name, change) => {
    const h = harness();
    const waiting = deferred<boolean>();
    let valid!: () => boolean;
    h.prepare.mockImplementation(async (_time, check) => { valid = check; return waiting.promise; });
    h.scheduler.request();
    await h.advance(750);
    expect(valid()).toBe(true);
    change(h.state);
    expect(valid()).toBe(false);
    h.scheduler.request();
    await h.advance(1000);
    expect(h.prepare).toHaveBeenCalledTimes(1);
    waiting.resolve(false);
    await h.settle();
    expect(h.cached.size).toBe(0);
  });

  it('rejects a late frame when playback starts without a scheduler request', async () => {
    const h = harness();
    const waiting = deferred<void>();
    h.prepare.mockImplementation(async (time, valid) => {
      await waiting.promise;
      if (!valid()) return false;
      h.cached.add(Math.round(time * h.state.fps));
      return true;
    });
    h.scheduler.request();
    await h.advance(750);
    h.state.blocked = true;
    waiting.resolve();
    await h.settle();
    expect(h.cached.size).toBe(0);
    expect(h.queued()).toHaveLength(0);
  });

  it('rechecks capacity before committing a frame if foreground work consumes the remaining RAM', async () => {
    const h = harness({ budget: 10 });
    const waiting = deferred<boolean>();
    let valid!: () => boolean;
    h.prepare.mockImplementation(async (_time, check) => { valid = check; return waiting.promise; });
    h.scheduler.request();
    await h.advance(750);
    h.state.bytes = 10;
    expect(valid()).toBe(false);
    waiting.resolve(false);
    await h.settle();
    expect(h.queued()).toHaveLength(0);
  });

  it('never overlaps preparations, and backs off by the cost of an expensive frame', async () => {
    const h = harness();
    const waiting = deferred<boolean>();
    h.prepare.mockImplementationOnce(() => waiting.promise);
    h.scheduler.request();
    await h.advance(750);
    for (let i = 0; i < 100; i++) h.scheduler.request();
    await h.advance(500);
    expect(h.prepare).toHaveBeenCalledTimes(1);
    expect(h.queued()).toHaveLength(0);
    waiting.resolve(true);
    await h.settle();
    await h.advance(499);
    expect(h.prepare).toHaveBeenCalledTimes(1);
    await h.advance(1);
    expect(h.prepare).toHaveBeenCalledTimes(2);
  });

  it('backs off failed work without unhandled rejections or overlapping retries', async () => {
    const h = harness();
    h.prepare.mockRejectedValue(new Error('Decode not ready'));
    h.scheduler.request();
    await h.advance(750);
    expect(h.prepare).toHaveBeenCalledTimes(1);
    await h.advance(999);
    expect(h.prepare).toHaveBeenCalledTimes(1);
    await h.advance(1);
    expect(h.prepare).toHaveBeenCalledTimes(2);
    await h.advance(1999);
    expect(h.prepare).toHaveBeenCalledTimes(2);
    await h.advance(1);
    expect(h.prepare).toHaveBeenCalledTimes(3);
  });

  it('reuses completed valid frames after cancellation and resumes without overlapping canceled work', async () => {
    const h = harness();
    h.scheduler.request();
    await h.advance(750);
    expect(h.cached.has(0)).toBe(true);
    const waiting = deferred<boolean>();
    let valid!: () => boolean;
    h.prepare.mockImplementationOnce(async (_time, check) => { valid = check; return waiting.promise; });
    await h.advance(50);
    h.scheduler.cancel();
    expect(valid()).toBe(false);
    expect(h.cached.has(0)).toBe(true);
    h.scheduler.request();
    await h.advance(750);
    expect(h.prepare).toHaveBeenCalledTimes(2);
    waiting.resolve(false);
    await h.settle();
    await h.advance(50);
    expect(h.prepare).toHaveBeenCalledTimes(3);
    expect(h.prepare.mock.calls.at(-1)![0]).toBe(1 / 24);
  });

  it('ignores a timer callback delivered after cancellation or disposal', async () => {
    const h = harness();
    h.scheduler.request();
    const canceled = h.timers[0]!;
    h.scheduler.cancel();
    canceled.callback();
    await h.settle();
    expect(h.prepare).not.toHaveBeenCalled();
    h.scheduler.request();
    const disposed = h.timers.at(-1)!;
    h.scheduler.dispose();
    disposed.callback();
    h.scheduler.request();
    await h.advance(10_000);
    expect(h.prepare).not.toHaveBeenCalled();
    expect(h.queued()).toHaveLength(0);
  });

  it('disposal invalidates an in-flight frame and never rearms after it finishes', async () => {
    const h = harness();
    const waiting = deferred<boolean>();
    let valid!: () => boolean;
    h.prepare.mockImplementation(async (_time, check) => { valid = check; return waiting.promise; });
    h.scheduler.request();
    await h.advance(750);
    h.scheduler.dispose();
    expect(valid()).toBe(false);
    waiting.resolve(false);
    await h.settle();
    h.scheduler.request();
    await h.advance(10_000);
    expect(h.prepare).toHaveBeenCalledTimes(1);
    expect(h.queued()).toHaveLength(0);
  });

  it('a canceled frame does not query resources which have already been disposed', async () => {
    const h = harness();
    let elapsed = 0;
    const state = vi.fn(() => h.state);
    const waiting = deferred<boolean>();
    let valid!: () => boolean;
    const scheduler = createAutomaticPreview({
      state, has: h.has, schedule: h.schedule, now: () => elapsed,
      prepare: async (_time, check) => { valid = check; return waiting.promise; },
    });
    scheduler.request();
    elapsed = 750;
    h.timers[0]!.callback();
    await h.settle();
    expect(valid()).toBe(true);
    scheduler.dispose();
    const reads = state.mock.calls.length;
    state.mockImplementation(() => { throw new Error('Disposed renderer'); });
    expect(valid()).toBe(false);
    expect(() => h.timers[0]!.callback()).not.toThrow();
    waiting.resolve(false);
    await h.settle();
    expect(state).toHaveBeenCalledTimes(reads);
  });

  it('can refill a previously full cache when older frames are removed', async () => {
    const h = harness({ budget: 20 });
    h.scheduler.request();
    await h.advance(1000);
    expect([...h.cached]).toEqual([0, 1]);
    h.cached.delete(0);
    h.state.bytes = 10;
    h.scheduler.request();
    await h.advance(50);
    expect([...h.cached]).toEqual([1, 0]);
    expect(h.state.bytes).toBe(20);
    expect(h.queued()).toHaveLength(0);
  });
});
