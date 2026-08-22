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
