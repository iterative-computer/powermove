const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const engineSource = fs.readFileSync(path.join(root, 'js/core/engine.js'), 'utf8');

function delayedVideo() {
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
    pause() {
      this.pauseCalls++;
      this.paused = true;
    },
  };
  return { el, finishPlay: () => finishPlay() };
}

function engine({ layer = null, media = null, work = [0, 10] } = {}) {
  const listeners = new Map();
  const frames = [];
  const audioCalls = [];
  const Audio = {
    start(time) { audioCalls.push(['start', time]); },
    pause() { audioCalls.push(['pause']); },
    seek(time) { audioCalls.push(['seek', time]); },
    tick(time) { audioCalls.push(['tick', time]); },
  };
  const assets = new Map();
  if (media) assets.set('asset-1', { el: media.el, dur: 10 });
  const PM = {
    proj: { dur: 10, fps: 30, work, shutter: 0.5, layers: layer ? [layer] : [] },
    assets: { get: id => assets.get(id) },
    Audio,
    sel: {},
    GL: { gl: null },
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    snapF: (value, fps) => Math.round(value * fps) / fps,
    active: (item, time) => time >= item.from && time < item.from + item.dur,
    invalidate() {},
    bus: {
      on(name, handler) {
        if (!listeners.has(name)) listeners.set(name, []);
        listeners.get(name).push(handler);
      },
      emit(name, value) {
        for (const handler of listeners.get(name) || []) handler(value);
      },
    },
  };
  vm.runInContext(engineSource, vm.createContext({
    window: { PM },
    console,
    performance: { now: () => 0 },
    requestAnimationFrame(handler) { frames.push(handler); return frames.length; },
    document: { createElement() { return {}; } },
  }), { filename: 'js/core/engine.js' });
  return {
    PM,
    audioCalls,
    runFrame(now = 16) {
      const frame = frames.shift();
      assert.ok(frame, 'an animation frame is queued');
      frame(now);
    },
  };
}

test('transport delegates audio start, timeline ticks, seeks, and pause to PM.Audio', () => {
  const { PM, audioCalls, runFrame } = engine();

  PM.play();
  assert.deepEqual(audioCalls, [['start', 0]]);

  runFrame(16);
  assert.deepEqual(audioCalls[1][0], 'tick');
  assert.equal(audioCalls[1][1], 0.016);

  PM.setTime(2, { raw: true });
  assert.deepEqual(audioCalls.at(-1), ['seek', 2]);

  PM.pause();
  assert.deepEqual(audioCalls.at(-1), ['pause']);
  PM.pause();
  assert.deepEqual(audioCalls.at(-1), ['pause'], 'Pause still clears audio when transport was already stopped');
  assert.equal(audioCalls.filter(([name]) => name === 'pause').length, 2);
});

test('looping seeks the audio scheduler before the new work-area tick', () => {
  const { PM, audioCalls, runFrame } = engine({ work: [1, 2] });
  PM.time = 1.99;

  PM.play();
  runFrame(32);

  assert.deepEqual(audioCalls, [
    ['start', 1.99],
    ['seek', 1],
    ['tick', 1],
  ]);
  assert.equal(PM.time, 1);
});

test('pause wins when a video start finishes late', async () => {
  const media = delayedVideo();
  const layer = { id: 'video-1', type: 'video', on: true, from: 0, dur: 10, d: { asset: 'asset-1', speed: 1, trim: 0 } };
  const { PM, audioCalls, runFrame } = engine({ layer, media });

  PM.play();
  runFrame();
  PM.pause();
  media.finishPlay();
  await new Promise(resolve => setImmediate(resolve));

  assert.equal(PM.playing, false);
  assert.equal(media.el.paused, true, 'a stale play completion must not restart video after pause');
  assert.deepEqual(audioCalls[0], ['start', 0], 'video transport still starts the independent audio scheduler');
  assert.deepEqual(audioCalls.at(-1), ['pause']);
});
