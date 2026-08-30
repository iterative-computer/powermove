import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './engine';

function delayedVideo(): any {
  let finishPlay: any;
  const el: any = {
    paused: true,
    playbackRate: 1,
    seeking: false,
    currentTime: 0,
    playCalls: 0,
    pauseCalls: 0,
    play() {
      this.playCalls++;
      return new Promise<void>(resolve => {
        finishPlay = () => { this.paused = false; resolve(); };
      });
    },
    pause() {
      this.pauseCalls++;
      this.paused = true;
    },
  };
  return { el, finishPlay: () => finishPlay() };
}

function engine({ layer = null, media = null, work = [0, 10] }: any = {}): any {
  const listeners = new Map<string, any[]>();
  const frames: any[] = [];
  let nowValue = 0;
  const audioCalls: any[] = [];
  const Audio = {
    start(time: any) { audioCalls.push(['start', time]); },
    pause() { audioCalls.push(['pause']); },
    seek(time: any) { audioCalls.push(['seek', time]); },
    tick(time: any) { audioCalls.push(['tick', time]); },
  };
  const assets = new Map<string, any>();
  if (media) assets.set('asset-1', { el: media.el, dur: 10 });
  const PM: PMRegistry = {
    proj: { dur: 10, fps: 30, work, shutter: 0.5, layers: layer ? [layer] : [] },
    assets: { get: (id: string) => assets.get(id) },
    Audio,
    sel: {},
    GL: { gl: null },
    clamp: (value: any, min: any, max: any) => Math.max(min, Math.min(max, value)),
    snapF: (value: any, fps: any) => Math.round(value * fps) / fps,
    active: (item: any, time: any) => time >= item.from && time < item.from + item.dur,
    invalidate() {},
    bus: {
      on(name: string, handler: any) {
        if (!listeners.has(name)) listeners.set(name, []);
        listeners.get(name)!.push(handler);
      },
      emit(name: string, value?: any) {
        for (const handler of listeners.get(name) || []) handler(value);
      },
    },
  };
  vi.stubGlobal('window', {
    performance: { now: () => nowValue },
    requestAnimationFrame(handler: any) { frames.push(handler); return frames.length; },
    document: { createElement() { return {}; } },
  });
  install(PM);
  return {
    PM,
    audioCalls,
    reinstall() {
      install(PM);
    },
    runFrame(now = 16) {
      nowValue = now;
      const frame = frames.shift();
      if (!frame) throw new Error('an animation frame is queued');
      frame(now);
    },
    runQueuedFrames(now = 16) {
      nowValue = now;
      const queued = frames.splice(0);
      if (!queued.length) throw new Error('an animation frame is queued');
      queued.forEach(frame => frame(now));
    },
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('legacy engine install', () => {
  it('delegates transport start, timeline tick, seek, and pause to PM.Audio', () => {
    const { PM, audioCalls, runFrame } = engine();

    PM.play();
    expect(audioCalls).toEqual([['start', 0]]);

    runFrame(16);
    expect(audioCalls[1]).toEqual(['tick', 0.016]);

    PM.setTime(2, { raw: true });
    expect(audioCalls.at(-1)).toEqual(['seek', 2]);

    PM.pause();
    PM.pause();
    expect(audioCalls.filter(([name]: any) => name === 'pause')).toHaveLength(2);
  });

  it('keeps one playback clock when the renderer bootstrap installs the engine again', () => {
    const { PM, reinstall, runQueuedFrames } = engine();

    reinstall();
    PM.play();
    runQueuedFrames(100);

    expect(PM.time).toBeCloseTo(0.1);
  });

  it('seeks the audio scheduler before the new work-area tick when looping', () => {
    const { PM, audioCalls, runFrame } = engine({ work: [1, 2] });
    PM.time = 1.99;

    PM.play();
    runFrame(32);

    expect(audioCalls).toEqual([
      ['start', 1.99],
      ['seek', 1],
      ['tick', 1],
    ]);
    expect(PM.time).toBe(1);
  });

  it('lets pause win when a video start finishes late', async () => {
    const media = delayedVideo();
    const layer = { id: 'video-1', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, audioCalls, runFrame } = engine({ layer, media });

    PM.play();
    runFrame();
    PM.pause();
    media.finishPlay();
    await Promise.resolve();

    expect(media.el.paused).toBe(true);
    expect(audioCalls.at(-1)).toEqual(['pause']);
  });

  it('resynchronizes a delayed decoder once without repeatedly seeking ordinary drift', async () => {
    const media = delayedVideo();
    const layer = { id: 'video-1', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, runFrame } = engine({ layer, media });

    PM.play();
    runFrame(250);
    media.el.currentTime = 0;
    media.finishPlay();
    await Promise.resolve();
    expect(media.el.currentTime).toBeCloseTo(PM.time);

    media.el.currentTime = 0;
    runFrame(750);
    expect(media.el.currentTime).toBe(0);
  });

  it('seeks a playing video after an explicit timeline jump', async () => {
    const media = delayedVideo();
    const layer = { id: 'video-1', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, runFrame } = engine({ layer, media });

    PM.play();
    runFrame(16);
    media.finishPlay();
    await Promise.resolve();
    PM.setTime(2, { raw: true });
    runFrame(32);

    expect(media.el.currentTime).toBeCloseTo(PM.time);
  });

  it('uses the layer speed as the video playback rate', () => {
    const media = delayedVideo();
    const layer = { id: 'video-1', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'asset-1', speed: 1.5, trim: 0 } };
    const { PM, runFrame } = engine({ layer, media });

    PM.play();
    runFrame(16);
    expect(media.el.playbackRate).toBe(1.5);
  });
});
