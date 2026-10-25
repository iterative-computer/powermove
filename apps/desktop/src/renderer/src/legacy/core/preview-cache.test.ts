import { afterEach, describe, expect, it, vi } from 'vitest';
import { installPreviewCache } from './preview-cache';

vi.mock('./frame-preparation', () => ({ prepareFrame: vi.fn(async () => {}) }));

function fixture() {
  const listeners = new Map<string, Array<() => void>>();
  const bitmap = { close: vi.fn() };
  const drawImage = vi.fn();
  const canvas = { style: {}, getContext: () => ({ drawImage }), remove: vi.fn() };
  vi.stubGlobal('document', { createElement: () => canvas });
  vi.stubGlobal('createImageBitmap', vi.fn(async () => bitmap));
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  const audio = { running: true, tick: vi.fn(), seek: vi.fn(), start: vi.fn(() => { audio.running = true; }), pause: vi.fn(() => { audio.running = false; }) };
  const PM: any = {
    proj: { id: 'project', w: 2, h: 2, fps: 30, dur: 1 / 30, work: [0, 1 / 30] },
    time: 0, playing: true, quality: 1, Audio: audio,
    GL: { canvas: { width: 2, height: 2, parentElement: { appendChild: vi.fn() } }, render: vi.fn() },
    pause: () => { PM.playing = false; audio.pause(); },
    setTime: vi.fn((time: number) => { PM.time = time; PM.bus.emit('time', time); }),
    invalidate: vi.fn(), toast: vi.fn(),
    bus: {
      on(event: string, run: () => void) { listeners.set(event, [...listeners.get(event) || [], run]); },
      emit(event: string) { for (const run of listeners.get(event) || []) run(); },
    },
  };
  installPreviewCache(PM);
  return { PM, audio, bitmap, canvas };
}

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('cached preview audio ownership', () => {
  it.each(['quality', 'layers', 'project'])('keeps normal playback audible when %s invalidates an inactive cache', event => {
    const { PM, audio } = fixture();
    PM.bus.emit(event);
    expect(PM.playing).toBe(true);
    expect(audio.running).toBe(true);
    expect(audio.pause).not.toHaveBeenCalled();
  });

  it('does not stop normal audio when stopping an inactive preview', () => {
    const { PM, audio } = fixture();
    PM.Preview.stop();
    expect(audio.running).toBe(true);
    expect(audio.pause).not.toHaveBeenCalled();
  });

  it.each(['clear', 'stop'])('%s stops audio owned by active cached playback', async action => {
    vi.useFakeTimers();
    const { PM, audio, bitmap, canvas } = fixture();
    const prepared = PM.Preview.cache();
    await vi.runAllTimersAsync();
    await prepared;
    expect(PM.Preview.active).toBe(true);
    expect(audio.running).toBe(true);
    audio.pause.mockClear();
    PM.Preview[action]();
    expect(PM.Preview.active).toBe(false);
    expect(audio.running).toBe(false);
    expect(audio.pause).toHaveBeenCalledTimes(1);
    expect(canvas.remove).toHaveBeenCalled();
    expect(bitmap.close).toHaveBeenCalledTimes(action === 'clear' ? 1 : 0);
    // Normal playback can resume without later cache invalidations muting it.
    PM.playing = true; audio.start(); audio.pause.mockClear();
    PM.bus.emit('quality');
    expect(audio.running).toBe(true);
    expect(audio.pause).not.toHaveBeenCalled();
  });
});

it.each(['stop', 'clear'])('renders native cache frames and releases native audio on %s', async action => {
  vi.useFakeTimers();
  const { PM, audio, canvas } = fixture();
  PM.proj.dur = 2; PM.proj.work = [1, 1 + 2 / 30];
  const source = { width: 2, height: 2 };
  const renderFrameTo = vi.fn(async () => source);
  const pictures = [{ close: vi.fn() }, { close: vi.fn() }];
  vi.mocked(createImageBitmap).mockImplementationOnce(async () => pictures[0] as any).mockImplementationOnce(async () => pictures[1] as any);
  const transport = { active: () => true, setLoop: vi.fn(), play: vi.fn(), pause: vi.fn(), now: vi.fn(() => ({ time: 1 + 1 / 30, playing: true })) };
  PM.EngineConnection = { render: { renderFrameTo }, transport };
  const prepared = PM.Preview.cache();
  await vi.runAllTimersAsync(); await prepared;
  expect(renderFrameTo.mock.calls).toEqual([
    [1, 2, 2, { mblur: true, mbSamples: 6 }],
    [1 + 1 / 30, 2, 2, { mblur: true, mbSamples: 6 }],
  ]);
  expect(PM.GL.render).not.toHaveBeenCalled();
  expect(vi.mocked(createImageBitmap).mock.calls.every(([value]) => value === source)).toBe(true);
  expect(transport.setLoop).toHaveBeenCalledWith([1, 1 + 2 / 30]);
  expect(transport.play).toHaveBeenCalledWith(1);
  expect(audio.start).not.toHaveBeenCalled();
  const tick = vi.mocked(requestAnimationFrame).mock.calls[0]![0];
  tick(performance.now() + 1000 / 60);
  expect(canvas.getContext().drawImage).toHaveBeenCalledWith(pictures[1], 0, 0);
  expect(PM.time).toBeCloseTo(1 + 1 / 30);
  expect(audio.tick).not.toHaveBeenCalled();
  PM.Preview[action]();
  expect(transport.pause).toHaveBeenCalledOnce();
  expect(transport.setLoop).toHaveBeenLastCalledWith(null);
  expect(audio.pause).toHaveBeenCalledTimes(2); // cache setup and playback handoff
});

it('sizes native cached frames from composition quality, ignoring the hidden legacy canvas', async () => {
  vi.useFakeTimers();
  const { PM, canvas } = fixture();
  PM.proj.w = 640; PM.proj.h = 360; PM.proj.work = [0, 2 / 30]; PM.quality = .25;
  PM.GL.canvas.width = 1464; PM.GL.canvas.height = 824;
  PM.Memory = { budget: () => 200_000 };
  const renderFrameTo = vi.fn(async () => {
    // The compatibility canvas can change while the native viewer presents.
    PM.GL.canvas.width = 1920;
    return { width: 160, height: 90 };
  });
  PM.EngineConnection = { render: { renderFrameTo }, transport: { active: () => false } };
  const prepared = PM.Preview.cache();
  await vi.runAllTimersAsync(); await prepared;
  expect(renderFrameTo.mock.calls).toEqual([
    [0, 160, 90, { mblur: true, mbSamples: 6 }],
    [1 / 30, 160, 90, { mblur: true, mbSamples: 6 }],
  ]);
  expect(PM.Preview.active).toBe(true);
  expect(PM.Preview.count).toBe(2);
  expect(PM.Preview.bytes).toBe(2 * 160 * 90 * 4);
  expect(canvas.width).toBe(160); expect(canvas.height).toBe(90);
  expect(PM.GL.canvas.parentElement.appendChild).toHaveBeenCalledWith(canvas);
  expect(PM.toast).not.toHaveBeenCalled();
  PM.Preview.stop();
});

it('restores the project loop after a native cached preview ends', async () => {
  vi.useFakeTimers();
  const { PM } = fixture();
  PM.proj.work = [0, 0.05]; PM.loop = true;
  const transport = { active: () => true, setLoop: vi.fn(), play: vi.fn(), pause: vi.fn(), now: vi.fn() };
  PM.EngineConnection = { render: { renderFrameTo: vi.fn(async () => ({ width: 2, height: 2 })) }, transport };
  const prepared = PM.Preview.cache();
  await vi.runAllTimersAsync(); await prepared;
  expect(transport.setLoop).toHaveBeenCalledWith([0, 2 / 30]);
  PM.Preview.stop();
  expect(transport.setLoop).toHaveBeenLastCalledWith([0, 0.05]);
});

it('starts at frame zero when the first RAF timestamp predates playback setup', async () => {
  vi.useFakeTimers();
  const { PM, bitmap, canvas, audio } = fixture();
  PM.proj.work = [0, 2 / 30];
  const prepared = PM.Preview.cache();
  await vi.runAllTimersAsync();
  await prepared;
  const tick = vi.mocked(requestAnimationFrame).mock.calls[0]![0];
  tick(performance.now() - 1);
  expect(canvas.getContext().drawImage).toHaveBeenCalledWith(bitmap, 0, 0);
  expect(PM.time).toBe(0);
  expect(audio.seek).not.toHaveBeenCalled();
});

it('starts the cached clock after synchronous audio setup has finished', async () => {
  vi.useFakeTimers();
  const { PM, audio } = fixture();
  PM.proj.dur = 1; PM.proj.work = [0, 1];
  audio.start.mockImplementation(() => { vi.advanceTimersByTime(250); });
  const prepared = PM.Preview.cache(); await vi.runAllTimersAsync(); await prepared;
  const tick = vi.mocked(requestAnimationFrame).mock.calls[0]![0];
  tick(performance.now());
  expect(PM.time).toBe(0);
  expect(audio.tick).toHaveBeenLastCalledWith(0);
});

it('advances the cached playhead on all 120 display ticks while choosing 30 accurate pictures', async () => {
  vi.useFakeTimers();
  const { PM, canvas, audio } = fixture();
  PM.proj.dur = 3; PM.proj.work = [1, 2];
  const pictures = Array.from({ length: 30 }, (_, frame) => ({ frame, close: vi.fn() }));
  let next = 0;
  vi.mocked(createImageBitmap).mockImplementation(async () => pictures[next++] as any);
  const prepared = PM.Preview.cache(); await vi.runAllTimersAsync(); await prepared;
  const origin = performance.now();
  const tick = vi.mocked(requestAnimationFrame).mock.calls[0]![0];
  const times: number[] = [];
  for (let display = 0; display < 120; display++) {
    tick(origin + display * 1000 / 120);
    times.push(PM.time);
    expect(canvas.getContext().drawImage).toHaveBeenLastCalledWith(pictures[Math.floor(display / 4)], 0, 0);
    expect(PM.time).toBeCloseTo(1 + display / 120, 10);
  }
  expect(new Set(times).size).toBe(120);
  expect(canvas.getContext().drawImage).toHaveBeenCalledTimes(30);
  expect(audio.tick).toHaveBeenCalledTimes(120);
  expect(audio.tick).toHaveBeenLastCalledWith(PM.time);
  expect(audio.seek).not.toHaveBeenCalled();
  PM.Preview.stop();
  expect(PM.setTime).toHaveBeenLastCalledWith(59 / 30, { raw: true });
  expect(PM.proj.fps).toBe(30);
});

it('updates the next cached clock before a queued timeline paint and keeps loop audio continuous', async () => {
  vi.useFakeTimers();
  const { PM, audio } = fixture();
  PM.proj.dur = 3; PM.proj.work = [1, 2];
  const prepared = PM.Preview.cache(); await vi.runAllTimersAsync(); await prepared;
  const origin = performance.now();
  const tick = vi.mocked(requestAnimationFrame).mock.calls[0]![0];
  const queue: FrameRequestCallback[] = [];
  vi.mocked(requestAnimationFrame).mockImplementation(callback => { queue.push(callback); return queue.length; });
  const painted: number[] = [];
  PM.invalidate.mockImplementation(() => requestAnimationFrame(() => painted.push(PM.time)));
  tick(origin);
  const nextDisplay = queue.splice(0);
  nextDisplay.forEach(callback => callback(origin + 1000 / 120));
  expect(painted).toEqual([1 + 1 / 120]);
  tick(origin + 1000 + 1000 / 120);
  expect(PM.time).toBeCloseTo(1 + 1 / 120, 10);
  expect(audio.seek).toHaveBeenLastCalledWith(PM.time);
});

it('respects Off for explicitly prepared previews as well', async () => {
  const { PM } = fixture();
  PM.Memory = { budget: () => 0 };
  await PM.Preview.cache();
  expect(createImageBitmap).not.toHaveBeenCalled();
  expect(PM.Preview.bytes).toBe(0);
  expect(PM.toast).toHaveBeenCalledWith(expect.stringContaining('Preview memory is off'), 6000);
});

it('discards a pending bitmap if the user disables memory while preparing', async () => {
  const { PM, bitmap } = fixture();
  let limit = 256 * 1024 * 1024;
  let finish!: (value: any) => void;
  PM.Memory = { budget: () => limit };
  vi.mocked(createImageBitmap).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const preparing = PM.Preview.cache();
  await Promise.resolve();
  limit = 0;
  finish(bitmap);
  await preparing;
  expect(bitmap.close).toHaveBeenCalledOnce();
  expect(PM.Preview.bytes).toBe(0);
  expect(PM.Preview.active).toBe(false);
});
