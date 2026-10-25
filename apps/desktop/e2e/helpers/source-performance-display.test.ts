import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { commonSourceTargets, installSourcePerformanceDisplay, sourceFrameIndex, sourceFrameTimeline, sourceTimelineCorrespondence, type SourceDisplayProbe, type SourceFrameTimeline } from './source-performance-display';

describe('authoritative source frame timing', () => {
  it('keeps actual variable frame timestamps, independently of nominal FPS', () => {
    expect(sourceFrameTimeline({ streams: [{ time_base: '1/1000', start_pts: 0 }],
      frames: [0, 40, 110].map(best_effort_timestamp => ({ best_effort_timestamp })) }))
      .toEqual({ pts: [0, .04, .11], quantum: .001 });
  });
  it('refuses missing, duplicate, shifted, or coarse timestamps instead of guessing', () => {
    for (const [start, pts] of [[0, [0, undefined]], [0, [0, 0]], [10, [10, 50]], [0, [0, 2]], [0, [0, 110, 40]]] as const)
      expect(() => sourceFrameTimeline({ streams: [{ time_base: '1/1000', start_pts: start }],
        frames: pts.map(best_effort_timestamp => ({ best_effort_timestamp })) })).toThrow();
  });
  it('validates ordinal correspondence against the actual rendition timebase', () => {
    const original = { pts: [0, 1 / 30, 2 / 30], quantum: 1 / 15360 };
    const rendition = { pts: [0, .033, .067], quantum: .001 };
    expect(sourceTimelineCorrespondence(original, rendition, [.01, .05, .08])).toMatchObject({ frames: 3, checkedTargets: 3 });
    expect(() => sourceTimelineCorrespondence(original, { ...rendition, pts: [0, .033] }, [])).toThrow('frame count');
    expect(() => sourceTimelineCorrespondence(original, { ...rendition, pts: [0, .034, .067] }, [])).toThrow('timestamp/order');
    expect(sourceTimelineCorrespondence(original, rendition, [.0665])).toMatchObject({ checkedTargets: 1 });
    expect(() => sourceTimelineCorrespondence(original, rendition, [2 / 30])).toThrow('different original/rendition frame ordinals');
  });
  it('selects exact half-open intervals and pairs boundary targets without future-frame epsilon', () => {
    const original = { pts: Array.from({ length: 60 }, (_, i) => i / 30), quantum: 1 / 15360 };
    const rendition = { pts: original.pts.map(pts => Math.round(pts * 1000) / 1000), quantum: .001 };
    expect(sourceFrameIndex(rendition, 29 / 30)).toBe(28);
    expect(sourceFrameIndex(rendition, .967)).toBe(29);
    const nominal = Array.from({ length: 24 }, (_, k) => Math.round((k * 1.9 / 24 + 1 / 60) * 705_600_000) / 705_600_000);
    const plan = commonSourceTargets(original, rendition, nominal);
    expect(plan).toHaveLength(24);
    for (const sample of plan) {
      expect(sourceFrameIndex(original, sample.target)).toBe(sourceFrameIndex(original, sample.nominal));
      expect(sourceFrameIndex(rendition, sample.target)).toBe(sample.frame);
      expect(sample.target).toBeGreaterThan(sample.lower);
      expect(sample.target).toBeLessThan(sample.upper);
      expect(Math.abs(sample.deltaMs)).toBeLessThan(.5);
      if (sample.nominal > sample.lower + sample.margin && sample.nominal < sample.upper - sample.margin)
        expect(sample.target).toBe(sample.nominal);
    }
    expect(sourceTimelineCorrespondence(original, rendition, plan.map(sample => sample.target))).toMatchObject({ checkedTargets: 24 });
  });
});

describe('source display endpoint', () => {
  let animation: FrameRequestCallback[], resized: (() => void)[], probes: SourceDisplayProbe[];
  const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
  const raf = async () => { const callbacks = animation.splice(0); callbacks.forEach(fn => fn(performance.now())); await flush(); };
  beforeEach(() => {
    vi.useFakeTimers(); animation = []; resized = []; probes = [];
    vi.stubGlobal('window', {});
    vi.stubGlobal('document', { hidden: false });
    vi.stubGlobal('devicePixelRatio', 1);
    vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => animation.push(fn));
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    vi.stubGlobal('ResizeObserver', class {
      constructor(fn: () => void) { resized.push(fn); }
      observe() {} disconnect() {}
    });
    installSourcePerformanceDisplay();
  });
  afterEach(() => { probes.forEach(probe => probe.dispose()); vi.useRealTimers(); vi.unstubAllGlobals(); });

  function setup(native = false, trace = false, timeline: SourceFrameTimeline = { pts: [0, .04, .11], quantum: .001 }) {
    const callbacks = new Map<number, VideoFrameRequestCallback>();
    let next = 0, at = 0, stamp = 0;
    const video = Object.assign(new EventTarget(), {
      isConnected: true, clientWidth: 320, clientHeight: 180, currentSrc: 'fixture', srcObject: {},
      paused: true, readyState: 2, load: vi.fn(),
      requestVideoFrameCallback(fn: VideoFrameRequestCallback) { callbacks.set(++next, fn); return next; },
      cancelVideoFrameCallback(id: number) { callbacks.delete(id); }
    });
    const emit = (mediaTime: number, presentationTime = performance.now()) => {
      const pending = [...callbacks]; callbacks.clear();
      for (const [, fn] of pending) fn(performance.now(), { mediaTime, presentationTime,
        expectedDisplayTime: performance.now(), presentedFrames: next, width: 160, height: 90 });
    };
    const waiting = new Map<number, () => void>();
    const presenter = {
      presentedTimestamp: -1,
      presentedAt(timestamp: number) {
        return new Promise<void>(resolve => {
          const timer = setTimeout(resolve, 250);
          waiting.set(timestamp, () => { clearTimeout(timer); resolve(); });
        });
      },
      async paint(descriptor: { timestamp: number }) { this.presentedAt(descriptor.timestamp); }
    };
    const monitor = native ? Object.assign(new EventTarget(), { surface: video, videoPresenter: presenter,
      requested: 0, asset: 'asset', rendering: false, paintAgain: false }) : video;
    Object.defineProperty(monitor, 'currentTime', { get: () => at, set: (time: number) => {
      at = time;
      if (native) (monitor as any).requested++;
      queueMicrotask(() => {
        if (native) void presenter.paint({ timestamp: ++stamp }).then(() => monitor.dispatchEvent(new Event('loadeddata')));
        else monitor.dispatchEvent(new Event('seeked'));
      });
    } });
    const probe: SourceDisplayProbe = (window as any).__sourcePerformanceDisplay(monitor, {
      native, asset: 'asset', timeline, trace
    });
    probes.push(probe);
    const present = () => { presenter.presentedTimestamp = stamp; emit(stamp / 1e6); waiting.get(stamp)?.(); };
    return { video, monitor, presenter, probe, emit, present };
  }

  it('waits past seeked and a wrong indexed PTS until the requested PTS plus RAF', async () => {
    const f = setup(); f.emit(0); await f.probe.ready();
    let done = false;
    const seek = f.probe.seek(.06).then(() => { done = true; });
    await flush(); f.emit(.11); await flush(); expect(done).toBe(false);
    f.emit(.04); await flush(); expect(done).toBe(false);
    await raf(); await seek; expect(done).toBe(true);
  });

  it('matches the real millisecond rendition PTS, without broadening the rVFC tolerance', async () => {
    const f = setup(false, false, { pts: [0, .033, .067], quantum: .001 });
    f.emit(0); await f.probe.ready();
    const seek = f.probe.seek(.05); await flush(); f.emit(.033); await flush(); await raf(); await seek;
    const wrong = expect(f.probe.seek(.08)).rejects.toThrow('unindexed PTS');
    await flush(); f.emit(2 / 30); await wrong;
  });

  it('reloads an already-painted paused legacy source once, then requires a real callback', async () => {
    const f = setup();
    let ready = false;
    const opening = f.probe.ready().then(() => { ready = true; });
    expect(f.video.load).toHaveBeenCalledTimes(1);
    await raf(); expect(ready).toBe(false); // readyState alone proves no display.
    f.emit(0, -1); await raf(); expect(ready).toBe(false); // Queued pre-load callback is stale.
    f.emit(0); await raf(); await opening; expect(ready).toBe(true);
  });

  it('fails startup if reloading still produces no presentation callback', async () => {
    const f = setup();
    const opening = expect(f.probe.ready()).rejects.toThrow('setup timed out');
    await vi.advanceTimersByTimeAsync(15_000); await opening;
    expect(f.video.load).toHaveBeenCalledTimes(1);
  });

  it('reuses a confirmed legacy frame only with unchanged source and geometry', async () => {
    const f = setup(); f.emit(0); await f.probe.ready();
    const same = f.probe.seek(.01); await flush(); await raf(); await same;
    f.video.clientWidth = 640; resized.forEach(fn => fn());
    let done = false;
    const resizedSeek = f.probe.seek(.01).then(() => { done = true; });
    await flush(); await raf(); expect(done).toBe(false);
    f.emit(0); await flush(); await raf(); await resizedSeek;
    f.video.currentSrc = 'replacement';
    await expect(f.probe.seek(.01)).rejects.toThrow('identity changed');
  });

  it('does not accept native loadeddata or an old stamp for the current delivery', async () => {
    const f = setup(true); (f.monitor as any).currentTime = 0;
    await flush(); f.present(); await flush(); await f.probe.ready();
    let done = false;
    const seek = f.probe.seek(.01).then(() => { done = true; });
    await flush(); f.emit(1 / 1e6); await flush(); await raf(); expect(done).toBe(false);
    f.present(); await flush(); expect(done).toBe(false);
    await raf(); await seek; expect(done).toBe(true);
  });

  it('fails the production presentation timeout instead of calling it an acknowledgement', async () => {
    const f = setup(true); (f.monitor as any).currentTime = 0;
    await flush(); f.present(); await flush(); await f.probe.ready();
    const seek = expect(f.probe.seek(.06)).rejects.toThrow('presentation bound is not an acknowledgement');
    await flush(); await vi.advanceTimersByTimeAsync(250); await seek;
  });

  it('bounds RAF waiting and trace storage without reading pixels', async () => {
    const f = setup(false, true); f.emit(0); await f.probe.ready();
    const seek = expect(f.probe.seek(.01)).rejects.toThrow('timed out');
    await flush(); // Matched frame, but no RAF is delivered.
    await vi.advanceTimersByTimeAsync(15_000); await seek;
    for (let i = 0; i < 600; i++) f.emit(0);
    expect(f.probe.evidence().events).toHaveLength(512);
    expect(f.probe.evidence().dropped).toBeGreaterThan(0);
  });
});
