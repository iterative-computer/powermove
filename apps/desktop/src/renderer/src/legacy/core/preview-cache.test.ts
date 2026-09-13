import { afterEach, describe, expect, it, vi } from 'vitest';
import { installPreviewCache } from './preview-cache';

vi.mock('./frame-preparation', () => ({ prepareFrame: vi.fn(async () => {}) }));

function fixture() {
  const listeners = new Map<string, Array<() => void>>();
  const bitmap = { close: vi.fn() };
  const canvas = { style: {}, getContext: () => ({ drawImage: vi.fn() }), remove: vi.fn() };
  vi.stubGlobal('document', { createElement: () => canvas });
  vi.stubGlobal('createImageBitmap', vi.fn(async () => bitmap));
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  const audio = { running: true, start: vi.fn(() => { audio.running = true; }), pause: vi.fn(() => { audio.running = false; }) };
  const PM: any = {
    proj: { id: 'project', fps: 30, dur: 1 / 30, work: [0, 1 / 30] },
    time: 0, playing: true, Audio: audio,
    GL: { canvas: { width: 2, height: 2, parentElement: { appendChild: vi.fn() } }, render: vi.fn() },
    pause: () => { PM.playing = false; audio.pause(); },
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
