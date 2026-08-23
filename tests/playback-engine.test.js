const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

function delayedMedia() {
  let finishPlay;
  const el = {
    paused: true,
    currentTime: 0,
    playCalls: 0,
    pauseCalls: 0,
    play() {
      this.playCalls++;
      return new Promise(resolve => {
        finishPlay = () => { this.paused = false; resolve(); };
      });
    },
    pause() { this.pauseCalls++; this.paused = true; },
    cloneNode() { return this; },
  };
  return { el, finishPlay: () => finishPlay() };
}

function immediateMedia() {
  const el = {
    paused: true,
    currentTime: 0,
    playCalls: 0,
    pauseCalls: 0,
    src: 'blob:test',
    play() { this.playCalls++; this.paused = false; return Promise.resolve(); },
    pause() { this.pauseCalls++; this.paused = true; },
    cloneNode() { return this; },
  };
  return { el };
}

function engine(layer, media) {
  const listeners = new Map();
  const frames = [];
  const PM = {
    proj: { dur: 10, fps: 30, work: [0, 10], shutter: 0.5, layers: [layer] },
    assets: new Map([['asset-1', { el: media.el, dur: 10 }]]),
    sel: {},
    GL: { gl: null },
    clamp: (v, min, max) => Math.max(min, Math.min(max, v)),
    snapF: (v, fps) => Math.round(v * fps) / fps,
    active: (item, time) => time >= item.from && time < item.from + item.dur,
    invalidate() {},
    bus: {
      on(name, fn) {
        if (!listeners.has(name)) listeners.set(name, []);
        listeners.get(name).push(fn);
      },
      emit(name, value) {
        for (const fn of listeners.get(name) || []) fn(value);
      },
    },
  };
  class AudioContext {
    constructor() { this.state = 'running'; this.destination = {}; }
    resume() { this.state = 'running'; return Promise.resolve(); }
    createMediaElementSource() { return { connect() { return this; }, disconnect() {} }; }
    createGain() { return { gain: { value: 1 }, connect() { return this; }, disconnect() {} }; }
  }
  const context = vm.createContext({
    window: { PM, AudioContext },
    console,
    performance: { now: () => 0 },
    requestAnimationFrame(fn) { frames.push(fn); return frames.length; },
    document: { createElement() { return {}; } },
  });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/engine.js'), 'utf8'), context);
  return { PM, runFrame: (now = 16) => frames.shift()(now) };
}

test('pause wins when an audio start finishes late', async () => {
  const media = delayedMedia();
  const { PM } = engine({ id: 'audio-1', type: 'audio', on: true, from: 0, dur: 10, d: { asset: 'asset-1' } }, media);

  PM.play();
  PM.pause();
  media.finishPlay();
  await new Promise(resolve => setImmediate(resolve));

  assert.equal(PM.playing, false);
  assert.equal(media.el.paused, true, 'a stale play completion must not restart audio after pause');
});

test('pause wins when a video start finishes late', async () => {
  const media = delayedMedia();
  const { PM, runFrame } = engine({ id: 'video-1', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'asset-1' } }, media);

  PM.play();
  runFrame();
  PM.pause();
  media.finishPlay();
  await new Promise(resolve => setImmediate(resolve));

  assert.equal(PM.playing, false);
  assert.equal(media.el.paused, true, 'a stale play completion must not restart video after pause');
});

test('audio starts when playback reaches a later clip and stops at its out point', () => {
  const media = immediateMedia();
  const layer = { id: 'audio-1', type: 'audio', on: true, solo: false, from: .5, dur: .5, d: { asset: 'asset-1', gain: 1 } };
  const { PM, runFrame } = engine(layer, media);

  PM.play();
  assert.equal(media.el.playCalls, 0, 'a future clip should stay silent before its in point');
  runFrame(250);
  runFrame(500);
  assert.equal(media.el.playCalls, 1, 'the engine should start audio as the playhead enters the clip');
  runFrame(750);
  runFrame(1000);
  assert.equal(media.el.paused, true, 'the engine should stop audio at the clip out point');
});

test('audio gain follows inspector gain and fade controls', () => {
  const media = immediateMedia();
  const layer = { id: 'audio-1', type: 'audio', on: true, solo: false, from: 0, dur: 10, d: { asset: 'asset-1', gain: 2, fadeIn: 2, fadeOut: 2 } };
  const { PM } = engine(layer, media);

  PM.setTime(1, { raw: true });
  PM.play();
  assert.equal(PM.audio.nodes.get(layer.id).gain.gain.value, 1, 'fade in should scale the layer gain');
  PM.setTime(9, { raw: true });
  assert.equal(PM.audio.nodes.get(layer.id).gain.gain.value, 1, 'fade out should scale the layer gain');
});

test('audio playback honors solo state', () => {
  const first = immediateMedia();
  const second = immediateMedia();
  const layers = [
    { id: 'audio-1', type: 'audio', on: true, solo: false, from: 0, dur: 10, d: { asset: 'asset-1', gain: 1 } },
    { id: 'audio-2', type: 'audio', on: true, solo: true, from: 0, dur: 10, d: { asset: 'asset-2', gain: 1 } },
  ];
  const listeners = new Map();
  const frames = [];
  const PM = {
    proj: { dur: 10, fps: 30, work: [0, 10], shutter: .5, layers },
    assets: new Map([['asset-1', { el: first.el, dur: 10 }], ['asset-2', { el: second.el, dur: 10 }]]),
    sel: {}, GL: { gl: null },
    clamp: (v, min, max) => Math.max(min, Math.min(max, v)), snapF: v => v,
    active: (item, time) => time >= item.from && time < item.from + item.dur,
    invalidate() {},
    bus: { on(name, fn) { if (!listeners.has(name)) listeners.set(name, []); listeners.get(name).push(fn); }, emit() {} },
  };
  class AudioContext {
    constructor() { this.state = 'running'; this.destination = {}; }
    resume() { return Promise.resolve(); }
    createMediaElementSource() { return { connect() { return this; }, disconnect() {} }; }
    createGain() { return { gain: { value: 1 }, connect() { return this; }, disconnect() {} }; }
  }
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/engine.js'), 'utf8'), vm.createContext({
    window: { PM, AudioContext }, console, performance: { now: () => 0 },
    requestAnimationFrame(fn) { frames.push(fn); }, document: { createElement() { return {}; } },
  }));

  PM.play();
  assert.equal(first.el.playCalls, 0, 'non-solo audio should stay silent');
  assert.equal(second.el.playCalls, 1, 'solo audio should play');
});
