import { afterEach, describe, expect, it, vi } from 'vitest';

import { createServicesRegistry } from '../../kernel/services';
import type { PMRegistry } from '../registry';
import { install } from './engine';
import * as videoInstances from './video-instances';

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

function engine({ layer = null, media = null, work = [0, 10], audioStartupDelay = 0 }: any = {}): any {
  const listeners = new Map<string, any[]>();
  const frames: any[] = [];
  let nowValue = 0;
  const audioCalls: any[] = [];
  const Audio = {
    start(time: any) { audioCalls.push(['start', time]); nowValue += audioStartupDelay; },
    pause() { audioCalls.push(['pause']); },
    seek(time: any) { audioCalls.push(['seek', time]); },
    tick(time: any) { audioCalls.push(['tick', time]); },
  };
  const assets = new Map<string, any>();
  if (media) assets.set('asset-1', { el: media.el, dur: 10 });
  const PM: PMRegistry = {
    Kernel: { services: createServicesRegistry() },
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
    assets,
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
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('legacy engine install', () => {
  it.each(['1', '0.5', '0.25'])('keeps the selected %s preview resolution through slow playback and paused refinement', (resolution) => {
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    const qualityChanged = vi.fn(); PM.bus.on('quality', qualityChanged);
    PM.setPreviewResolution(resolution);
    expect(PM.quality).toBe(Number(resolution));
    expect(PM.perf.auto).toBe(false);
    expect(qualityChanged).toHaveBeenCalledTimes(1);
    PM.play();
    for (let frame = 1; frame < 100; frame++) runFrame(frame * 1000 / 12);
    PM.pause(); PM.bus.emit('draw'); runFrame(10000);
    expect(PM.quality).toBe(Number(resolution));
    PM.setPreviewResolution('auto');
    expect(PM.quality).toBe(1); expect(PM.perf.auto).toBe(true);
  });
  it('updates paused scrub values without rebuilding structural UI', () => {
    const { PM } = engine();
    const invalidate = vi.spyOn(PM, 'invalidate'), time = vi.fn();
    PM.bus.on('time', time);
    PM.setTime(2);
    expect(time).toHaveBeenCalledWith(2);
    expect(invalidate.mock.calls.map(([kind]: any[]) => kind)).toEqual(['render', 'timeline', 'status']);
  });
  it('does not invalidate a cached loop by raising quality based on its cheap replay cost', () => {
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(); PM.GL.stats = { previewHit: true };
    PM.quality = .5; PM.perf.auto = true;
    PM.play(); runFrame(1000);
    expect(PM.quality).toBe(.5);
    PM.GL.stats.previewHit = false;
    runFrame(2000);
    expect(PM.quality).toBe(.5);
  });

  it('reduces GPU-bound playback to Quarter despite cheap CPU submissions and restores Full when paused', () => {
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true); PM.GL.stats = { previewHit: false };
    for (let frame = 1; frame <= 10; frame++) runFrame(frame * 1000 / 60);
    PM.play();
    for (let frame = 1; frame <= 140; frame++) runFrame(1000 / 6 + frame * 1000 / 15);
    expect(PM.perf.ms).toBe(0);
    expect(PM.quality).toBe(.25);
    // Healthy playback after adaptation does not pump resolution back up.
    for (let frame = 1; frame <= 180; frame++) runFrame(9500 + frame * 1000 / 60);
    expect(PM.quality).toBe(.25);
    PM.pause(); PM.bus.emit('draw'); runFrame(12700);
    expect(PM.quality).toBe(1);
  });

  it.each([[30, 120], [120, 60]])('keeps healthy %i fps projects at Full on %i Hz displays', (fps, displayHz) => {
    const { PM, runFrame } = engine();
    PM.proj.fps = fps;
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    for (let frame = 1; frame <= 10; frame++) runFrame(frame * 1000 / displayHz);
    PM.play();
    for (let frame = 1; frame <= displayHz * 5; frame++) runFrame((10 + frame) * 1000 / displayHz);
    expect(PM.quality).toBe(1);
  });

  it('recognizes very slow GPU frames instead of repeatedly treating them as startup', () => {
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    PM.play();
    for (let frame = 1; frame <= 15; frame++) runFrame(frame * 1000 / 3);
    expect(PM.quality).toBeLessThan(1);
  });

  it('does not mistake unsuccessful decoder frames or repeated seeks for GPU pressure', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.play();
    for (let frame = 1; frame <= 90; frame++) runFrame(frame * 1000 / 15);
    expect(PM.quality).toBe(1);
    PM.GL.render.mockReturnValue(true);
    for (let frame = 91; frame <= 180; frame++) {
      if (frame % 5 === 0) PM.setTime(2);
      runFrame(frame * 1000 / 15);
    }
    expect(PM.quality).toBe(1);
  });

  it('preserves manual preview resolution under sustained low playback cadence', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    PM.perf.auto = false; PM.quality = .75;
    PM.play();
    for (let frame = 1; frame <= 90; frame++) runFrame(frame * 1000 / 15);
    expect(PM.quality).toBe(.75);
    PM.pause(); PM.bus.emit('draw'); runFrame(6500);
    expect(PM.quality).toBe(.75);
  });

  it('excludes cached replay and shader warmup from cadence-driven quality changes', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true); PM.GL.stats = { previewHit: true };
    PM.play();
    for (let frame = 1; frame <= 75; frame++) runFrame(frame * 1000 / 15);
    expect(PM.quality).toBe(1);
    PM.GL.stats.previewHit = false; PM.GL.compiling = 3;
    for (let frame = 76; frame <= 150; frame++) runFrame(frame * 1000 / 15);
    expect(PM.quality).toBe(1);
  });

  it('starts a fresh cadence window after a long background gap', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    PM.play();
    for (let frame = 1; frame <= 30; frame++) runFrame(frame * 1000 / 15);
    expect(PM.quality).toBe(1);
    runFrame(10000);
    for (let frame = 1; frame <= 30; frame++) runFrame(10000 + frame * 1000 / 15);
    expect(PM.quality).toBe(1);
  });

  it('holds non-linear overlapping videos at one composition time until the complete frame is presented', () => {
    const foreground = delayedVideo(), background = delayedVideo();
    const layer = { id: 'foreground', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1, trim: 1, timeRemap: true } };
    const { PM, assets, runFrame } = engine({ layer, media: foreground });
    assets.set('asset-2', { el: background.el, dur: 10 });
    PM.proj.layers.push({ ...layer, id: 'background', d: { asset: 'asset-2', speed: 2, trim: .5 } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.play(); runFrame(100); runFrame(200);
    expect(PM.GL.render.mock.calls.map(([time]: number[]) => time)).toEqual([.1, .1]);
    expect(foreground.el.currentTime).toBeCloseTo(1.1);
    expect(background.el.currentTime).toBeCloseTo(.7);
    expect(foreground.el.playCalls + background.el.playCalls).toBe(0);
    // The transport/audio clock keeps running; a slow decode only drops video
    // frames, and the next request catches up after the held frame is drawn.
    expect(PM.time).toBe(.2);
    PM.GL.render.mockReturnValue(true); runFrame(210); runFrame(240);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBeCloseTo(7 / 30);
    expect(foreground.el.currentTime).toBeCloseTo(1 + 7 / 30);
  });

  it('discards a pending synchronized frame on a timeline jump and resumes streaming after the overlap', () => {
    const foreground = delayedVideo(), background = delayedVideo();
    const layer = { id: 'foreground', type: 'video', from: 0, dur: 1, d: { asset: 'asset-1', speed: 1, trim: 0, timeRemap: true } };
    const { PM, assets, runFrame } = engine({ layer, media: foreground });
    assets.set('asset-2', { el: background.el, dur: 10 });
    PM.proj.layers.push({ ...layer, id: 'background', dur: 3, d: { ...layer.d, asset: 'asset-2', timeRemap: false } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.play(); runFrame(100);
    PM.setTime(.5, { raw: true }); runFrame(116);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBe(.5);
    PM.setTime(1.5, { raw: true }); runFrame(132);
    expect(PM.videoFrameSync).toBe(false);
    expect(background.el.playCalls).toBe(1);
    expect(foreground.el.paused).toBe(true);
  });

  it('abandons an unfinished synchronized frame when playback passes the overlap end', () => {
    const foreground = delayedVideo(), background = delayedVideo();
    const layer = { id: 'foreground', type: 'video', from: 0, dur: 1, d: { asset: 'asset-1', speed: 1, trim: 0, timeRemap: true } };
    const { PM, assets, runFrame } = engine({ layer, media: foreground });
    assets.set('asset-2', { el: background.el, dur: 10 });
    PM.proj.layers.push({ ...layer, id: 'background', dur: 3, d: { ...layer.d, asset: 'asset-2', timeRemap: false } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.play(); runFrame(100);
    for (let now = 200; now <= 1200; now += 100) runFrame(now);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBe(1.2);
    expect(PM.videoFrameSync).toBe(false);
    expect(background.el.playCalls).toBe(1);
    expect(foreground.el.paused).toBe(true);
  });

  it('starts the playback clock after synchronous audio device setup', () => {
    const { PM, runFrame, audioCalls } = engine({ audioStartupDelay: 250 });
    PM.time = 119 / 30;
    PM.play();
    runFrame(266);
    expect(PM.time).toBeCloseTo(119 / 30 + .016);
    expect(audioCalls).toContainEqual(['start', 119 / 30]);
    expect(audioCalls.filter((call: any[]) => call[0] === 'seek')).toEqual([]);
  });

  it('starts the visible decoder at Play without waiting for a display tick', () => {
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 10, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM } = engine({ layer, media });
    PM.setTime(2, { raw: true });
    PM.play();
    expect(media.el.playCalls).toBe(1);
    expect(media.el.currentTime).toBe(2);
  });

  it('prepares the live decoder even when a paused composition uses saved pixels', () => {
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 10, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, runFrame } = engine({ layer, media });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    PM.setTime(2, { raw: true }); runFrame(16);
    expect(media.el.currentTime).toBe(2);
    expect(media.el.playCalls).toBe(0);
  });

  it('presents completed scrub frames while the pointer keeps moving and settles at the latest time', () => {
    const { PM, runFrame } = engine();
    let decoded = -1;
    const shown: number[] = [];
    PM.GL.gl = {};
    PM.GL.render = vi.fn((time: number) => {
      if (time !== decoded) return false;
      shown.push(time); return true;
    });
    PM.setTime(1, { raw: true }); runFrame(16);
    PM.setTime(2, { raw: true }); runFrame(32);
    decoded = 1;
    PM.setTime(3, { raw: true }); runFrame(48);
    expect(shown).toEqual([1]);
    runFrame(64);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBe(3);
    decoded = 3; runFrame(80);
    expect(shown).toEqual([1, 3]);
    expect(PM.time).toBe(3);
  });

  it('abandons an unfinished scrub frame when the composition changes', () => {
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.setTime(1, { raw: true }); runFrame(16);
    PM.setTime(3, { raw: true }); PM.bus.emit('layers'); runFrame(32);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBe(3);
  });

  it.each([1.2, 2.2])('abandons an unfinished outgoing video when scrubbing to %s after its cut', (target) => {
    const outgoing = delayedVideo(), incoming = delayedVideo();
    const layer = { id: 'outgoing', type: 'video', from: 0, dur: 1, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, assets, runFrame } = engine({ layer, media: outgoing });
    assets.set('asset-2', { el: incoming.el, dur: 10 });
    PM.proj.layers.push({ ...layer, id: 'incoming', from: 1, d: { ...layer.d, asset: 'asset-2' } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.setTime(.8, { raw: true }); runFrame(16);
    PM.setTime(target, { raw: true }); runFrame(32);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBe(target);
    expect(outgoing.el.currentTime).toBe(.8);
    if (target < 2) expect(incoming.el.currentTime).toBeCloseTo(target - 1);
  });

  it('abandons an unfinished incoming video when scrubbing backwards across its cut', () => {
    const outgoing = delayedVideo(), incoming = delayedVideo();
    const layer = { id: 'outgoing', type: 'video', from: 0, dur: 1, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, assets, runFrame } = engine({ layer, media: outgoing });
    assets.set('asset-2', { el: incoming.el, dur: 10 });
    PM.proj.layers.push({ ...layer, id: 'incoming', from: 1, d: { ...layer.d, asset: 'asset-2' } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => false);
    PM.setTime(1.2, { raw: true }); runFrame(16);
    PM.setTime(.8, { raw: true }); runFrame(32);
    expect(PM.GL.render.mock.calls.at(-1)[0]).toBe(.8);
    expect(outgoing.el.currentTime).toBe(.8);
  });

  it('does not render the same project frame twice on a high refresh display', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn(); PM.animVersion = () => 1;
    PM.play(); runFrame(1); runFrame(8); runFrame(16); runFrame(24);
    expect(PM.GL.render).toHaveBeenCalledTimes(1);
    runFrame(40);
    expect(PM.GL.render).toHaveBeenCalledTimes(2);
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    PM.bus.emit('quality'); runFrame(48);
    expect(PM.GL.render).toHaveBeenCalledTimes(3);
  });

  it('plans video decoders once per presented project frame while audio keeps ticking at display refresh', () => {
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1 } };
    const { PM, runFrame, audioCalls } = engine({ layer, media });
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    const active = vi.spyOn(PM, 'active');
    PM.play(); runFrame(1);
    const firstFrameChecks = active.mock.calls.length;
    expect(firstFrameChecks).toBeGreaterThan(0);
    runFrame(8); runFrame(16); runFrame(24);
    expect(active).toHaveBeenCalledTimes(firstFrameChecks);
    expect(audioCalls.filter(([name]: any[]) => name === 'tick')).toHaveLength(4);
    expect(PM.time).toBe(.024);
    runFrame(40);
    expect(active.mock.calls.length).toBeGreaterThan(firstFrameChecks);
    expect(PM.GL.render).toHaveBeenCalledTimes(2);
  });

  it('keeps unchanged decoder ownership through playback, pause and scrubbing without repruning it', async () => {
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 5, d: { asset: 'asset-1', speed: 1 } };
    const { PM, runFrame } = engine({ layer, media });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    PM.invalidate = () => PM.bus.emit('draw');
    const prune = vi.spyOn(videoInstances, 'pruneVideoInstances');
    PM.play(); media.finishPlay(); await Promise.resolve();
    for (const time of [40, 80, 120]) runFrame(time);
    PM.pause();
    PM.setTime(1); runFrame(140);
    PM.setTime(2); runFrame(160);
    expect(media.el.paused).toBe(true);
    expect(media.el.currentTime).toBe(2);
    expect(prune).toHaveBeenCalledTimes(1);
  });

  it('updates decoder ownership for lookahead, cuts, gaps and loop entrance', () => {
    const outgoing = delayedVideo(), incoming = delayedVideo();
    const layer = { id: 'outgoing', type: 'video', from: 0, dur: 1, d: { asset: 'asset-1', speed: 1 } };
    const { PM, assets, runFrame } = engine({ layer, media: outgoing, work: [0, 3] });
    assets.set('asset-2', { el: incoming.el, dur: 10 });
    PM.proj.layers.push({ ...layer, id: 'incoming', from: 1, d: { ...layer.d, asset: 'asset-2' } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    const prune = vi.spyOn(videoInstances, 'pruneVideoInstances');
    const owned = () => prune.mock.calls.map(([asset, ids]: any[]) => ({
      asset: asset === assets.get('asset-1') ? 'outgoing' : 'incoming', ids: [...ids],
    }));
    PM.play(); runFrame(100); runFrame(600); runFrame(700); runFrame(1100); runFrame(2100); runFrame(2600); runFrame(2700); runFrame(3100);
    expect(owned()).toEqual([
      { asset: 'outgoing', ids: ['outgoing'] },
      { asset: 'incoming', ids: ['incoming'] },
      { asset: 'outgoing', ids: [] },
      { asset: 'incoming', ids: [] },
      { asset: 'outgoing', ids: ['outgoing'] },
    ]);
    expect(outgoing.el.currentTime).toBe(0);
    expect(incoming.el.paused).toBe(true);
    expect(outgoing.el.playCalls).toBeGreaterThan(0);
  });

  it('replaces a decoder whose editing proxy changes while ownership stays the same', async () => {
    const original = delayedVideo(), preview = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 5, d: { asset: 'asset-1', speed: 1 } };
    const { PM, assets, runFrame } = engine({ layer, media: original });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    const prune = vi.spyOn(videoInstances, 'pruneVideoInstances');
    PM.play(); original.finishPlay(); await Promise.resolve(); runFrame(40);
    assets.get('asset-1').preview = { el: preview.el, deferred: true };
    runFrame(80);
    expect(preview.el.playCalls).toBe(0);
    PM.pause();
    expect(original.el.paused).toBe(true);
    expect(videoInstances.layerVideoElement(PM, assets.get('asset-1'), 'video')).toBe(preview.el);
    expect(preview.el.currentTime).toBeCloseTo(PM.time, 6);
    PM.play();
    expect(preview.el.playCalls).toBe(1);
    expect(prune).toHaveBeenCalledTimes(1);
  });

  it('releases a shared media instance when scrubbing across cuts with the same decoder count', () => {
    const media = delayedVideo();
    const copy = Object.assign(new EventTarget(), delayedVideo().el, { load: vi.fn(), removeAttribute: vi.fn() });
    vi.stubGlobal('document', { createElement: vi.fn(() => copy) });
    const layer = { id: 'first', type: 'video', from: 0, dur: 1, d: { asset: 'asset-1', speed: 1 } };
    const { PM, assets, runFrame } = engine({ layer, media });
    PM.proj.layers.push({ ...layer, id: 'second', from: 1, d: { ...layer.d, trim: 2 } });
    PM.GL.gl = {}; PM.GL.render = vi.fn(() => true);
    PM.invalidate = () => PM.bus.emit('draw');
    const prune = vi.spyOn(videoInstances, 'pruneVideoInstances');
    PM.setTime(.5); runFrame(16);
    PM.setTime(1.5); runFrame(32);
    expect(videoInstances.layerVideoElement(PM, assets.get('asset-1'), 'second')).toBe(copy);
    expect(copy.currentTime).toBe(2.5);
    PM.setTime(.5); runFrame(48);
    expect(videoInstances.layerVideoElement(PM, assets.get('asset-1'), 'first')).toBe(media.el);
    expect(copy.removeAttribute).toHaveBeenCalledWith('src');
    expect(copy.load).toHaveBeenCalledOnce();
    expect(prune.mock.calls.map(([, ids]: any[]) => [...ids])).toEqual([['first'], ['second'], ['first']]);
  });

  it('retries an unfinished decode within the same project frame instead of skipping it', () => {
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1 } };
    const { PM, runFrame } = engine({ layer, media });
    PM.GL.gl = {}; PM.GL.render = vi.fn().mockReturnValueOnce(false).mockReturnValueOnce(false).mockReturnValue(true);
    PM.play(); runFrame(1); runFrame(8); runFrame(16); runFrame(24);
    expect(PM.GL.render.mock.calls.map(([time]: number[]) => time)).toEqual([0, 0, 0]);
  });

  it('replans an already presented frame after a seek, frame-rate change or quality change', () => {
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1 } };
    const { PM, runFrame } = engine({ layer, media });
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    PM.play(); runFrame(1);
    PM.setTime(0, { raw: true, force: true }); runFrame(2);
    PM.previewFps = 60; runFrame(3);
    PM.bus.emit('quality'); runFrame(4);
    runFrame(5);
    expect(PM.GL.render.mock.calls.map(([time]: number[]) => time)).toEqual([0, 0, 0, 0]);
  });

  it('replans the same project frame when a short work area loops', () => {
    const media = delayedVideo();
    const layer = { id: 'video', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1 } };
    const { PM, runFrame, audioCalls } = engine({ layer, media, work: [0, .03] });
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    PM.play(); runFrame(1); runFrame(32);
    expect(PM.GL.render.mock.calls.map(([time]: number[]) => time)).toEqual([0, 0]);
    expect(audioCalls.filter(([name]: any[]) => name === 'seek')).toHaveLength(1);
  });

  it('resolves a shared upcoming cut once instead of rescanning the timeline for every clip', () => {
    const { PM, assets, runFrame } = engine();
    for (let i = 0; i < 6; i++) {
      const asset = `asset-${i}`;
      assets.set(asset, { el: delayedVideo().el, dur: 10 });
      PM.proj.layers.push({ id: `video-${i}`, type: 'video', from: .3, dur: 2, d: { asset, speed: 1, trim: 1 } });
    }
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    const active = vi.spyOn(PM, 'active');
    PM.play(); active.mockClear(); runFrame(1);
    // One scan at the current frame, one at the shared cut, and one active
    // check per upcoming layer. Repeated planning would grow quadratically.
    expect(active.mock.calls.length).toBeLessThanOrEqual(18);
    for (const asset of assets.values()) expect(asset.el.currentTime).toBe(1);
  });

  it('keeps a navigation redraw pending until refinement and never defers playback', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    const viewer = { deferNavigationRender: vi.fn(() => true) };
    PM.Kernel.services.register('viewer', viewer);
    runFrame(16); runFrame(32);
    expect(PM.GL.render).not.toHaveBeenCalled();
    viewer.deferNavigationRender.mockReturnValue(false);
    runFrame(48);
    expect(PM.GL.render).toHaveBeenCalledTimes(1);
    runFrame(64);
    expect(PM.GL.render).toHaveBeenCalledTimes(1);
    viewer.deferNavigationRender.mockReturnValue(true);
    PM.play(); runFrame(80);
    expect(PM.GL.render).toHaveBeenCalledTimes(2);
  });

  it('does not report sparse paused redraws as low playback FPS', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    runFrame(10000);
    PM.bus.emit('draw'); runFrame(11000);
    expect(PM.perf.fps).toBe(0);
  });

  it('starts a fresh FPS sample on playback and clears it when paused', () => {
    const { PM, runFrame } = engine();
    PM.GL.gl = {}; PM.GL.render = vi.fn();
    runFrame(10000);
    PM.play();
    for (let frame = 1; frame <= 32; frame++) runFrame(10000 + frame * 16);
    expect(PM.perf.fps).toBeGreaterThanOrEqual(28);
    expect(PM.perf.fps).toBeLessThanOrEqual(32);
    PM.pause();
    expect(PM.perf.fps).toBe(0);
    runFrame(30000); PM.play();
    for (let frame = 1; frame <= 32; frame++) runFrame(30000 + frame * 16);
    expect(PM.perf.fps).toBeGreaterThanOrEqual(28);
    expect(PM.perf.fps).toBeLessThanOrEqual(32);
  });

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

  it('keeps playback synchronized across a long background frame gap',()=>{
    const {PM,audioCalls,runFrame}=engine();PM.play();runFrame(100);runFrame(4100);
    expect(PM.time).toBeCloseTo(4.1);expect(audioCalls.filter((c:any)=>c[0]==='seek').at(-1)).toEqual(['seek',4.1]);
    runFrame(4116);expect(PM.time).toBeCloseTo(4.116);expect(audioCalls.filter((c:any)=>c[0]==='seek')).toHaveLength(1);
  });
  it('preserves elapsed time across several work-area loops while backgrounded',()=>{
    const {PM,runFrame}=engine({work:[1,2]});PM.time=1.5;PM.play();runFrame(3750);expect(PM.time).toBeCloseTo(1.25);
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
      ['seek', 1.0219999999999998],
      ['tick', 1.0219999999999998],
    ]);
    expect(PM.time).toBeCloseTo(1.022);
  });

  it.each([true, false])('inactive copies cannot pause a shared active video (inactive first: %s)', async first => {
    const media = delayedVideo();
    const layer = { id: 'active', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const inactive = { ...layer, id: 'later', from: 2 };
    const { PM, runFrame } = engine({ layer, media });
    PM.proj.layers = first ? [inactive, layer] : [layer, inactive];
    PM.play(); runFrame(16); media.finishPlay(); await Promise.resolve(); runFrame(32);
    expect(media.el.paused).toBe(false);
    expect(media.el.pauseCalls).toBe(0);
    expect(media.el.playCalls).toBe(1);
  });

  it('keeps the outgoing decoder active until its last displayed frame ends', async () => {
    const media = delayedVideo();
    const layer = { id: 'outgoing', type: 'video', from: 0, dur: .05, d: { asset: 'asset-1', speed: 1, trim: 0 } };
    const { PM, runFrame } = engine({ layer, media });
    PM.play(); runFrame(16); media.finishPlay(); await Promise.resolve();
    runFrame(51); // The displayed frame is still 1/30, inside this clip.
    expect(media.el.paused).toBe(false);
    runFrame(70);
    expect(media.el.paused).toBe(true);
  });

  it('prepares an upcoming trimmed clip without playing it early', () => {
    const media = delayedVideo();
    const layer = { id: 'incoming', type: 'video', from: .3, dur: 2, d: { asset: 'asset-1', speed: 1, trim: 1, timeRemap: true } };
    const { PM, runFrame } = engine({ layer, media });
    PM.play(); runFrame(16);
    expect(media.el.currentTime).toBe(1);
    expect(media.el.playCalls).toBe(0);
  });

  it('presents a decoded retimed frame before requesting the next seek', () => {
    const media = delayedVideo();
    const layer = { id: 'retimed', type: 'video', from: 0, dur: 2, d: { asset: 'asset-1', speed: 1, trim: 0, timeRemap: true } };
    const { PM, runFrame } = engine({ layer, media });
    PM.GL.gl = {};
    const presented: number[] = [];
    PM.GL.render = () => presented.push(media.el.currentTime);
    PM.play(); runFrame(100);
    expect(presented).toEqual([0]);
    expect(media.el.currentTime).toBeCloseTo(.1);
    runFrame(200);
    expect(presented).toEqual([0, .1]);
    expect(media.el.currentTime).toBeCloseTo(.2);
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
    expect(media.el.currentTime).toBeCloseTo(Math.floor(PM.time * PM.proj.fps) / PM.proj.fps);

    media.el.currentTime = 0;
    runFrame(116);
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

    expect(media.el.currentTime).toBeCloseTo(Math.floor(PM.time * PM.proj.fps) / PM.proj.fps);
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

it('pauses on the displayed project frame instead of leaving a fractional time that keys round forward', () => {
  const { PM, runFrame } = engine();
  PM.play();
  runFrame(1590);
  expect(PM.time).toBeCloseTo(1.59);
  PM.pause();
  expect(PM.time).toBe(47 / 30);
  expect(PM.snapF(PM.time, PM.proj.fps)).toBe(47 / 30);
});

it('keeps transport and audio stopped until project media finishes restoring', () => {
  const { PM, audioCalls } = engine();
  PM.assets.loading = new Set(['cutout']); PM.toast = vi.fn();
  PM.play();
  expect(PM.playing).toBe(false);
  expect(audioCalls).toEqual([]);
  expect(PM.toast).toHaveBeenCalledWith('Loading project media…', 3000);
  PM.assets.loading.clear(); PM.play();
  expect(PM.playing).toBe(true);
  expect(audioCalls[0][0]).toBe('start');
});
