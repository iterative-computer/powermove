const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const audioSource = fs.readFileSync(path.join(root, 'js/core/audio.js'), 'utf8');
const exporterSource = fs.readFileSync(path.join(root, 'js/core/exporter.js'), 'utf8');

class FakeAudioBuffer {
  constructor(channels, sampleRate = 8) {
    this._channels = channels.map(channel => Float32Array.from(channel));
    this.numberOfChannels = this._channels.length;
    this.length = this._channels[0]?.length || 0;
    this.sampleRate = sampleRate;
    this.duration = this.length / sampleRate;
  }

  getChannelData(channel) {
    return this._channels[channel];
  }
}

class FakeAudioParam {
  constructor(value = 1) {
    this.value = value;
    this.events = [];
    this.curves = [];
  }

  cancelScheduledValues(at) {
    this.events.push({ method: 'cancel', at });
  }

  setValueAtTime(value, at) {
    this.value = value;
    this.events.push({ method: 'set', value, at });
  }

  linearRampToValueAtTime(value, at) {
    this.value = value;
    this.events.push({ method: 'linear', value, at });
  }

  setValueCurveAtTime(curve, at, duration) {
    const values = Array.from(curve);
    this.value = values.at(-1);
    this.curves.push({ values, at, duration });
    this.events.push({ method: 'curve', values, at, duration });
  }
}

class FakeGainNode {
  constructor() {
    this.gain = new FakeAudioParam();
    this.connectedTo = null;
    this.disconnectCalls = 0;
  }

  connect(target) {
    this.connectedTo = target;
    return target;
  }

  disconnect() {
    this.disconnectCalls++;
  }
}

class FakeBufferSource {
  constructor() {
    this.buffer = null;
    this.connectedTo = null;
    this.starts = [];
    this.stopCalls = 0;
    this.disconnectCalls = 0;
    this.onended = null;
  }

  connect(target) {
    this.connectedTo = target;
    return target;
  }

  start(when, offset, duration) {
    this.starts.push({ when, offset, duration });
  }

  stop() {
    this.stopCalls++;
  }

  disconnect() {
    this.disconnectCalls++;
  }
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function flush(count = 4) {
  for (let index = 0; index < count; index++) {
    await new Promise(resolve => setImmediate(resolve));
  }
}

function closeTo(actual, expected, epsilon = 1e-6) {
  assert.ok(Math.abs(actual - expected) <= epsilon,
    `expected ${actual} to be within ${epsilon} of ${expected}`);
}

function harness({ decode } = {}) {
  const listeners = new Map();
  const assets = new Map();
  const contexts = [];
  const offlineContexts = [];
  const decodeCalls = [];

  const PM = {
    proj: { dur: 12, fps: 30, work: [0, 12], layers: [], comps: {}, assets: {} },
    assets: {
      map: assets,
      get(id) { return assets.get(id); },
    },
    time: 0,
    playing: false,
    bus: {
      on(name, handler) {
        if (!listeners.has(name)) listeners.set(name, []);
        listeners.get(name).push(handler);
      },
      emit(name, value) {
        for (const handler of listeners.get(name) || []) handler(value);
      },
    },
    invalidate() {},
    toast() {},
  };

  class AudioContext {
    constructor() {
      this.currentTime = 10;
      this.state = 'running';
      this.destination = { kind: 'destination' };
      this.sources = [];
      this.gains = [];
      contexts.push(this);
    }

    createBufferSource() {
      const source = new FakeBufferSource();
      this.sources.push(source);
      return source;
    }

    createGain() {
      const gain = new FakeGainNode();
      this.gains.push(gain);
      return gain;
    }

    resume() {
      this.state = 'running';
      return Promise.resolve();
    }

    decodeAudioData(bytes, success, failure) {
      decodeCalls.push(bytes);
      const pending = Promise.resolve().then(() => decode(bytes));
      pending.then(success, failure);
      return pending;
    }
  }

  class OfflineAudioContext {
    constructor(channels, length, sampleRate) {
      this.numberOfChannels = channels;
      this.length = length;
      this.sampleRate = sampleRate;
      this.destination = { kind: 'offline-destination' };
      this.sources = [];
      this.gains = [];
      this.rendered = new FakeAudioBuffer(
        Array.from({ length: channels }, () => new Float32Array(length)),
        sampleRate,
      );
      offlineContexts.push(this);
    }

    createBufferSource() {
      const source = new FakeBufferSource();
      this.sources.push(source);
      return source;
    }

    createGain() {
      const gain = new FakeGainNode();
      this.gains.push(gain);
      return gain;
    }

    startRendering() {
      return Promise.resolve(this.rendered);
    }
  }

  const document = { documentElement: { dataset: {} } };
  const window = { PM, AudioContext, OfflineAudioContext };
  vm.runInContext(audioSource, vm.createContext({
    window,
    document,
    console,
    Blob,
    Date,
    Promise,
    ArrayBuffer,
    DataView,
    Float32Array,
    Uint8Array,
    setTimeout,
    clearTimeout,
  }), { filename: 'js/core/audio.js' });

  return { PM, assets, contexts, offlineContexts, decodeCalls };
}

function asset(buffer, overrides = {}) {
  return {
    id: 'asset-1',
    name: 'audio.wav',
    kind: 'audio',
    dur: buffer.duration,
    channels: buffer.numberOfChannels,
    sampleRate: buffer.sampleRate,
    audioBlob: new Blob([new Uint8Array([82, 73, 70, 70])], { type: 'audio/wav' }),
    audioBuffer: buffer,
    audioDecoding: null,
    audioDisposed: false,
    audioToken: Symbol('asset-1'),
    peaks: null,
    ...overrides,
  };
}

function layer(overrides = {}) {
  return {
    id: 'audio-1',
    name: 'Audio',
    type: 'audio',
    on: true,
    solo: false,
    from: 0,
    dur: 5,
    d: { asset: 'asset-1', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 },
    ...overrides,
    d: {
      asset: 'asset-1', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0,
      ...(overrides.d || {}),
    },
  };
}

test('one lazy decode completion creates one voice even after repeated demand', async () => {
  const pending = deferred();
  const buffer = new FakeAudioBuffer([new Float32Array(40)], 10);
  const h = harness({ decode: () => pending.promise });
  const lazy = asset(buffer, { audioBuffer: null });
  h.assets.set(lazy.id, lazy);
  h.PM.proj.layers = [layer({ dur: 4 })];

  h.PM.Audio.start(0);
  h.PM.Audio.tick(.05);
  h.PM.Audio.tick(.1);
  h.PM.Audio.drawWaveform({ fillRect() {} }, h.PM.proj.layers[0], {
    x: 0, y: 0, width: 100, height: 20,
  });
  await flush(2);

  assert.equal(h.decodeCalls.length, 1, 'all lazy consumers share the same decoder promise');
  h.PM.time = .2;
  pending.resolve(buffer);
  await flush(6);

  assert.equal(h.contexts[0].sources.length, 1,
    'duplicate completion callbacks must not repeatedly stop and recreate the same voice');
  assert.equal(h.PM.Audio.inspect().voices.length, 1);
  closeTo(h.contexts[0].sources[0].starts[0].offset, .2);
});

test('a normal clock tick keeps its voice, but meaningful transport drift restarts at the corrected offset', () => {
  const buffer = new FakeAudioBuffer([new Float32Array(100)], 10);
  const h = harness({ decode: () => buffer });
  h.assets.set('asset-1', asset(buffer));
  h.PM.proj.layers = [layer({ dur: 8 })];

  h.PM.Audio.start(1);
  const context = h.contexts[0];
  assert.equal(context.sources.length, 1);

  context.currentTime = 10.5;
  h.PM.Audio.tick(1.5);
  assert.equal(context.sources.length, 1, 'matching transport and audio clocks do not churn the source');

  context.currentTime = 10.6;
  h.PM.Audio.tick(2.1);
  assert.equal(context.sources.length, 2, 'drift beyond tolerance restarts the source');
  assert.equal(context.sources[0].stopCalls, 1);
  closeTo(context.sources[1].starts[0].offset, 2.1);
});

test('exact automation preserves a sub-frame fade and fades at effective source EOF', () => {
  const buffer = new FakeAudioBuffer([new Float32Array(1000)], 1000); // source EOF at one second
  const h = harness({ decode: () => buffer });
  h.assets.set('asset-1', asset(buffer));
  const audio = layer({ dur: 5, d: { fadeIn: .001, fadeOut: .25 } });
  h.PM.proj.layers = [audio];

  assert.equal(typeof h.PM.Audio.envelopePoints, 'function');
  const points = Array.from(h.PM.Audio.envelopePoints(audio, 0, 1, 1), point => ({
    local: Number(point.local.toFixed(6)),
    value: Number(point.value.toFixed(6)),
  }));
  assert.deepEqual(points, [
    { local: 0, value: 0 },
    { local: .001, value: 1 },
    { local: .75, value: 1 },
    { local: 1, value: 0 },
  ]);

  h.PM.Audio.start(0);
  const gain = h.contexts[0].sources[0].connectedTo.gain;
  assert.equal(gain.curves.length, 0, 'short fades use exact automation points, not a sampled curve');
  const values = gain.events.filter(event => event.method !== 'cancel');
  const hasPoint = (method, value, at) => values.some(event => event.method === method
    && Math.abs(event.value - value) <= 1e-6 && Math.abs(event.at - at) <= 1e-6);
  assert.ok(hasPoint('set', 0, 10));
  assert.ok(hasPoint('linear', 1, 10.001), 'the one-millisecond fade-in keeps its exact boundary');
  assert.ok(values.some(event => event.value === 1 && Math.abs(event.at - 10.75) <= 1e-6),
    'the effective fade-out begins 250 ms before source EOF');
  assert.ok(hasPoint('linear', 0, 11), 'gain reaches silence exactly at source EOF');
});

test('audio nested in a precomp is discovered and shares clipped timing in playback and export', async () => {
  const buffer = new FakeAudioBuffer([new Float32Array(100)], 10);
  const h = harness({ decode: () => buffer });
  h.assets.set('asset-1', asset(buffer));
  h.PM.proj.layers = [{
    id: 'precomp-1', type: 'precomp', on: true, solo: false, from: 2, dur: 4,
    d: { comp: 'inner' },
  }];
  h.PM.proj.comps = {
    inner: {
      layers: [layer({ id: 'nested-audio', from: 1, dur: 5, d: { trim: .25 } })],
    },
  };

  assert.equal(h.PM.Audio.hasAudibleLayers(h.PM.proj), true);

  h.PM.Audio.start(3.5);
  const live = h.PM.Audio.inspect().voices[0];
  assert.equal(live.layerId, 'precomp-1/nested-audio');
  closeTo(live.sourceOffset, .75);
  closeTo(live.duration, 2.5, 1e-6);

  h.PM.Audio.pause();
  await h.PM.Audio.renderOffline(0, 8);
  const scheduled = h.offlineContexts[0].sources[0].starts[0];
  closeTo(scheduled.when, 3);
  closeTo(scheduled.offset, .25);
  closeTo(scheduled.duration, 3);
});

test('realtime recorder export explicitly routes the audio mix into its MediaStream', () => {
  const start = exporterSource.indexOf('async function exportRecorder');
  const end = exporterSource.indexOf('async function exportPNGs', start);
  assert.ok(start >= 0 && end > start, 'realtime recorder implementation is present');
  const recorder = exporterSource.slice(start, end);

  assert.match(audioSource, /createRealtimeMix/,
    'the audio engine exposes a recorder-specific mix destination');
  assert.match(audioSource, /supportsOpus/,
    'the export router can distinguish deterministic Opus support from recorder fallback');
  assert.match(exporterSource, /PM\.Audio\.supportsOpus\s*\(/,
    'WebM routing checks whether the deterministic path can encode audio');
  assert.match(recorder, /PM\.Audio\.createRealtimeMix\s*\(\s*t0\s*,\s*t1/,
    'realtime capture requests the same composition range from the audio engine');
  assert.match(recorder, /getAudioTracks\s*\(\)/);
  assert.match(recorder, /\.addTrack\s*\(/,
    'the recorder stream receives the audio track instead of remaining video-only');
  assert.match(recorder, /\.(?:stop|dispose|close)\s*\(/,
    'the temporary realtime mix is explicitly cleaned up');
});
