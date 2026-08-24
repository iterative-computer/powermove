const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const audioSource = fs.readFileSync(path.join(root, 'js/core/audio.js'), 'utf8');

class FakeAudioBuffer {
  constructor(channels, sampleRate = 8) {
    this._channels = channels.map(channel => Float32Array.from(channel));
    this.numberOfChannels = this._channels.length;
    this.length = this._channels[0]?.length || 0;
    this.sampleRate = sampleRate;
    this.duration = this.length / this.sampleRate;
    assert.ok(this._channels.every(channel => channel.length === this.length), 'fixture channels must have equal lengths');
  }

  getChannelData(channel) {
    return this._channels[channel];
  }
}

class FakeAudioParam {
  constructor(value = 1) {
    this.value = value;
    this.cancellations = [];
    this.curves = [];
    this.values = [];
    this.ramps = [];
  }

  cancelScheduledValues(at) {
    this.cancellations.push(at);
  }

  setValueCurveAtTime(curve, at, duration) {
    const values = Array.from(curve);
    this.curves.push({ values, at, duration });
    this.value = values.at(-1);
  }

  setValueAtTime(value, at) {
    this.values.push({ value, at });
    this.value = value;
  }

  linearRampToValueAtTime(value, at) {
    this.ramps.push({ value, at });
    this.value = value;
  }
}

class FakeGainNode {
  constructor(owner) {
    this.owner = owner;
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
  constructor(owner) {
    this.owner = owner;
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

function flush() {
  return new Promise(resolve => setImmediate(resolve));
}

function audioHarness({ decodedBuffer, decode } = {}) {
  const listeners = new Map();
  const events = [];
  const invalidations = [];
  const toasts = [];
  const contexts = [];
  const offlineContexts = [];
  const decodeCalls = [];
  const assets = new Map();

  const PM = {
    proj: { dur: 12, fps: 30, work: [0, 12], layers: [], assets: {} },
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
        events.push([name, value]);
        for (const handler of listeners.get(name) || []) handler(value);
      },
    },
    invalidate(name) { invalidations.push(name); },
    toast(message) { toasts.push(message); },
  };

  class AudioContext {
    constructor() {
      this.currentTime = 0.25;
      this.state = 'running';
      this.destination = { kind: 'destination' };
      this.sources = [];
      this.gains = [];
      this.resumeCalls = 0;
      contexts.push(this);
    }

    createBufferSource() {
      const source = new FakeBufferSource(this);
      this.sources.push(source);
      return source;
    }

    createGain() {
      const gain = new FakeGainNode(this);
      this.gains.push(gain);
      return gain;
    }

    createMediaStreamDestination() {
      const track = { stopped: false, stop() { this.stopped = true; } };
      return {
        track,
        stream: {
          getAudioTracks: () => [track],
          getTracks: () => [track],
        },
      };
    }

    resume() {
      this.resumeCalls++;
      this.state = 'running';
      return Promise.resolve();
    }

    decodeAudioData(bytes, success, failure) {
      decodeCalls.push(bytes);
      const pending = Promise.resolve().then(() => decode ? decode(bytes) : decodedBuffer);
      pending.then(value => success && success(value), error => failure && failure(error));
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
      const source = new FakeBufferSource(this);
      this.sources.push(source);
      return source;
    }

    createGain() {
      const gain = new FakeGainNode(this);
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

  return {
    PM,
    assets,
    contexts,
    offlineContexts,
    decodeCalls,
    events,
    invalidations,
    toasts,
    document,
  };
}

function audioAsset(buffer, id = 'asset-1') {
  return {
    id,
    name: id + '.wav',
    kind: 'audio',
    dur: buffer.duration,
    channels: buffer.numberOfChannels,
    sampleRate: buffer.sampleRate,
    audioBlob: new Blob([new Uint8Array([82, 73, 70, 70])], { type: 'audio/wav' }),
    audioBuffer: buffer,
    audioDecoding: null,
    audioDisposed: false,
    audioToken: Symbol(id),
    peaks: null,
  };
}

function audioLayer(overrides = {}) {
  const data = { asset: 'asset-1', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0, ...(overrides.d || {}) };
  return {
    id: 'audio-1',
    name: 'Audio',
    type: 'audio',
    on: true,
    solo: false,
    from: 0,
    dur: 4,
    ...overrides,
    d: data,
  };
}

function closeTo(actual, expected, epsilon = 1e-6) {
  assert.ok(Math.abs(actual - expected) <= epsilon, `expected ${actual} to be within ${epsilon} of ${expected}`);
}

test('audio file acceptance uses MIME first and a deliberate extension fallback', () => {
  const { PM } = audioHarness();

  assert.equal(PM.Audio.accepts({ name: 'recording.bin', type: 'audio/wav' }), true);
  assert.equal(PM.Audio.accepts({ name: 'renamed.MP3', type: 'application/octet-stream' }), true);
  assert.equal(PM.Audio.accepts({ name: 'mix.aiff', type: '' }), true);
  assert.equal(PM.Audio.accepts({ name: 'frame.png', type: 'image/png' }), false);
  assert.equal(PM.Audio.accepts({ name: 'movie.mp4', type: 'video/mp4' }), false);
  assert.equal(PM.Audio.accepts({ name: '', type: '' }), false);
});

test('preparing an import decodes actual channel samples and derives reusable waveform peaks', async () => {
  const buffer = new FakeAudioBuffer([
    [0, 0.25, -0.8, 0.1, 0.2, -0.4, 0.1, 0],
    [-0.1, 0.5, 0.3, -0.2, 0.9, 0.1, 0, -0.1],
  ], 4);
  const harness = audioHarness({ decodedBuffer: buffer });
  const blob = new Blob([new Uint8Array([82, 73, 70, 70, 1, 2, 3, 4])], { type: 'audio/wav' });

  const asset = await harness.PM.Audio.prepareAsset({ id: 'asset-1', name: 'landmarks.wav', blob });

  assert.equal(asset.audioBuffer, buffer);
  assert.equal(asset.audioBlob, blob);
  assert.equal(asset.dur, 2);
  assert.equal(asset.channels, 2);
  assert.equal(asset.sampleRate, 4);
  assert.deepEqual(Array.from(asset.peaks, value => Number(value.toFixed(6))), [0.1, 0.5, 0.8, 0.2, 0.9, 0.4, 0.1, 0.1]);
  assert.equal(harness.decodeCalls.length, 1);
  assert.ok(harness.events.some(([name, id]) => name === 'audio:decoded' && id === 'asset-1'));
  assert.ok(harness.invalidations.includes('timeline'));

  assert.equal(await harness.PM.Audio.decodeAsset(asset), buffer, 'the decoded buffer is cached');
  assert.equal(harness.decodeCalls.length, 1, 'cached waveform access never decodes twice');

  harness.assets.set(asset.id, asset);
  const bars = [];
  const painted = harness.PM.Audio.drawWaveform({ fillRect: (...args) => bars.push(args) }, audioLayer({ dur: 2 }), {
    x: 0, y: 0, width: 20, height: 12, clipLeft: 0, step: 2,
  });
  assert.equal(painted, true);
  assert.ok(bars.some(([, y, , height]) => height > 1 && y < 6), 'decoded peaks, not a synthetic texture, determine bar height');
});

test('oversized decoded audio is rejected before it can remain in the project cache', async () => {
  const huge = {
    numberOfChannels: 2,
    length: 42_000_000,
    sampleRate: 48_000,
    duration: 875,
    getChannelData() { throw new Error('peak generation must not run for an oversized buffer'); },
  };
  const harness = audioHarness({ decodedBuffer: huge });
  const blob = new Blob([new Uint8Array([1, 2, 3, 4])], { type: 'audio/wav' });

  await assert.rejects(
    harness.PM.Audio.prepareAsset({ id: 'asset-huge', name: 'too-long.wav', blob }),
    /too long to decode safely/,
  );
});

test('waveforms stop at source EOF instead of painting repeated terminal samples', () => {
  const buffer = new FakeAudioBuffer([[.2, .4, .8, .1]], 2); // two seconds
  const harness = audioHarness();
  const asset = { ...audioAsset(buffer), peaks: new Float32Array([.2, .4, .8, .1]) };
  harness.assets.set(asset.id, asset);
  const bars = [];

  harness.PM.Audio.drawWaveform({ fillRect: (...args) => bars.push(args) }, audioLayer({ dur: 4 }), {
    x: 0, y: 0, width: 40, height: 12, clipLeft: 0, step: 10,
  });

  assert.equal(bars.length, 2, 'only the audible first half of the extended layer is painted');
});

test('the clip planner shares trim, requested range, source bounds, enablement, and solo rules', () => {
  const buffer = new FakeAudioBuffer([new Float32Array(20)], 4); // five seconds
  const { PM } = audioHarness();
  const asset = { ...audioAsset(buffer), dur: 99 };
  const layer = audioLayer({ from: 2, dur: 6, d: { trim: 1 } });

  const clip = PM.Audio.plan(layer, asset, 3.5, 10, { solo: false });
  closeTo(clip.start, 3.5);
  closeTo(clip.localStart, 1.5);
  closeTo(clip.sourceOffset, 2.5);
  closeTo(clip.duration, 2.5);
  closeTo(clip.end, 6);

  assert.equal(PM.Audio.plan(layer, asset, 0, 2), null, 'a range ending at the clip in-point is empty');
  assert.equal(PM.Audio.plan({ ...layer, on: false }, asset, 0, 10), null);
  assert.equal(PM.Audio.plan(layer, asset, 0, 10, { solo: true }), null, 'global solo excludes a non-solo layer');
  assert.ok(PM.Audio.plan({ ...layer, solo: true }, asset, 0, 10, { solo: true }));
  assert.equal(PM.Audio.plan(audioLayer({ from: 2, d: { trim: 5 } }), asset, 3, 6), null,
    'a trim beyond decoded source duration cannot schedule a voice');
});

test('buffer-source playback starts once, restarts at exact seeks, pauses, and can start again', () => {
  const buffer = new FakeAudioBuffer([new Float32Array(40)], 4); // ten seconds
  const harness = audioHarness();
  const asset = audioAsset(buffer);
  const layer = audioLayer({ from: 2, dur: 5, d: { trim: 1 } });
  harness.assets.set(asset.id, asset);
  harness.PM.proj.layers = [layer];
  harness.PM.time = 3;

  harness.PM.Audio.start(3);
  const context = harness.contexts[0];
  assert.equal(context.sources.length, 1);
  assert.deepEqual(context.sources[0].starts[0], { when: 0.25, offset: 2, duration: 4 });
  assert.deepEqual(JSON.parse(JSON.stringify(harness.PM.Audio.inspect().voices)), [{
    layerId: 'audio-1', assetId: 'asset-1', sourceOffset: 2, duration: 4,
  }]);

  context.currentTime += .5;
  harness.PM.Audio.tick(3.5);
  assert.equal(context.sources.length, 1, 'normal clock ticks do not churn buffer sources');

  harness.PM.Audio.seek(5);
  assert.equal(context.sources[0].stopCalls, 1);
  assert.equal(context.sources.length, 2);
  assert.deepEqual(context.sources[1].starts[0], { when: 0.75, offset: 4, duration: 2 });

  harness.PM.Audio.pause();
  assert.equal(context.sources[1].stopCalls, 1);
  assert.equal(harness.PM.Audio.inspect().running, false);
  assert.equal(harness.PM.Audio.inspect().voices.length, 0);

  harness.PM.Audio.start(4);
  assert.equal(context.sources.length, 3);
  assert.deepEqual(context.sources[2].starts[0], { when: 0.75, offset: 3, duration: 3 });
  harness.PM.Audio.tick(7);
  assert.equal(context.sources[2].stopCalls, 1, 'the clip is stopped exactly at its out-point');
  assert.equal(harness.PM.Audio.inspect().voices.length, 0);
});

test('a future clip is scheduled at its in-point instead of sounding at transport start', () => {
  const buffer = new FakeAudioBuffer([new Float32Array(40)], 4); // ten seconds
  const harness = audioHarness();
  const asset = audioAsset(buffer);
  const layer = audioLayer({ from: 1.5, dur: 4, d: { trim: 1 } });
  harness.assets.set(asset.id, asset);
  harness.PM.proj.layers = [layer];

  harness.PM.Audio.start(1);

  const source = harness.contexts[0].sources[0];
  assert.deepEqual(source.starts[0], {
    when: 0.75,
    offset: 1,
    duration: 4,
  });
  assert.equal(source.connectedTo.gain.values[0].at, 0.75,
    'the gain envelope begins on the same audio-clock instant as the source');
});

test('gain and fade envelopes remain deterministic when fades overlap', () => {
  const { PM } = audioHarness();
  const layer = audioLayer({ dur: 4, d: { gain: 2, fadeIn: 3, fadeOut: 3 } });

  closeTo(PM.Audio.gainAt(layer, 0), 0);
  closeTo(PM.Audio.gainAt(layer, 1), 2 / 3);
  closeTo(PM.Audio.gainAt(layer, 2), 4 / 3);
  closeTo(PM.Audio.gainAt(layer, 3), 2 / 3);
  closeTo(PM.Audio.gainAt(layer, 4), 0);
  assert.deepEqual(Array.from(PM.Audio.envelope(layer, 0, 4, 5)).map(value => Number(value.toFixed(6))),
    [0, 0.666667, 1.333333, 0.666667, 0]);
  assert.equal(PM.Audio.gainAt(audioLayer({ d: { gain: 20 } }), 2), 4, 'gain is clamped to the public maximum');
});

test('a decode finishing after Pause cannot resurrect a stale playback generation', async () => {
  const pending = deferred();
  const buffer = new FakeAudioBuffer([new Float32Array(16)], 4);
  const harness = audioHarness({ decode: () => pending.promise });
  const asset = {
    ...audioAsset(buffer),
    dur: 4,
    audioBuffer: null,
    peaks: null,
  };
  const layer = audioLayer({ dur: 4 });
  harness.assets.set(asset.id, asset);
  harness.PM.proj.layers = [layer];

  harness.PM.Audio.start(0);
  await flush();
  const runningGeneration = harness.PM.Audio.inspect().generation;
  assert.equal(harness.decodeCalls.length, 1);
  harness.PM.Audio.pause();
  pending.resolve(buffer);
  await flush();
  await flush();

  assert.ok(harness.PM.Audio.inspect().generation > runningGeneration);
  assert.equal(harness.PM.Audio.inspect().running, false);
  assert.equal(harness.PM.Audio.inspect().voices.length, 0);
  assert.equal(harness.contexts[0].sources.length, 0, 'the stale decode never creates a source');

  harness.PM.Audio.start(1);
  assert.equal(harness.contexts[0].sources.length, 1, 'the cached decode remains usable for an explicit later start');
  assert.deepEqual(harness.contexts[0].sources[0].starts[0], { when: 0.25, offset: 1, duration: 3 });
});

test('a failed lazy decode backs off instead of retrying and toasting every frame', async () => {
  const harness = audioHarness({ decode: () => { throw new Error('decoder failed'); } });
  const buffer = new FakeAudioBuffer([new Float32Array(16)], 4);
  const asset = { ...audioAsset(buffer), audioBuffer: null, peaks: null, dur: 4 };
  harness.assets.set(asset.id, asset);
  harness.PM.proj.layers = [audioLayer({ dur: 4 })];

  harness.PM.Audio.start(0);
  harness.PM.Audio.tick(.05);
  await flush();
  await flush();
  harness.PM.Audio.tick(.1);
  harness.PM.Audio.tick(.15);
  await flush();

  assert.equal(harness.decodeCalls.length, 1);
  assert.equal(harness.toasts.length, 1);
  assert.ok(asset.audioRetryAt > Date.now());
});

test('disposing an asset invalidates a decode that is still in flight', async () => {
  const pending = deferred();
  const buffer = new FakeAudioBuffer([new Float32Array(8)], 4);
  const harness = audioHarness({ decode: () => pending.promise });
  const asset = {
    ...audioAsset(buffer),
    audioBuffer: null,
    peaks: null,
  };

  const decoding = harness.PM.Audio.decodeAsset(asset);
  await flush();
  harness.PM.Audio.disposeAsset(asset);
  pending.resolve(buffer);

  assert.equal(await decoding, null);
  assert.equal(asset.audioBuffer, null);
  assert.equal(asset.peaks, null);
  assert.equal(asset.audioBlob, null);
});

test('offline export schedules the same source offset and fade envelope as preview planning', async () => {
  const buffer = new FakeAudioBuffer([new Float32Array(40), new Float32Array(40)], 4); // ten seconds
  const harness = audioHarness();
  const asset = audioAsset(buffer);
  const layer = audioLayer({
    from: 2,
    dur: 6,
    d: { trim: 1, gain: 1.5, fadeIn: 3, fadeOut: 2 },
  });
  harness.assets.set(asset.id, asset);
  harness.PM.proj.layers = [layer];

  const preview = harness.PM.Audio.plan(layer, asset, 1, 6, { solo: false });
  const rendered = await harness.PM.Audio.renderOffline(1, 6);
  const offline = harness.offlineContexts[0];
  const source = offline.sources[0];
  const scheduled = source.starts[0];
  const gain = source.connectedTo.gain;
  const points = harness.PM.Audio.envelopePoints(layer, preview.localStart, preview.duration, preview.audibleDuration);

  assert.equal(rendered, offline.rendered);
  assert.equal(offline.length, 5 * 48000);
  assert.deepEqual(scheduled, {
    when: preview.start - 1,
    offset: preview.sourceOffset,
    duration: preview.duration,
  });
  assert.deepEqual(gain.values[0], { value: points[0].value, at: preview.start - 1 });
  assert.deepEqual(JSON.parse(JSON.stringify(gain.ramps)), Array.from(points.slice(1), point => ({
    value: point.value,
    at: preview.start - 1 + point.local - preview.localStart,
  })));
});

test('requested audio export fails visibly instead of silently dropping a missing or broken track', async () => {
  const decoded = new FakeAudioBuffer([new Float32Array(16)], 4);
  const missingHarness = audioHarness({ decodedBuffer: decoded });
  missingHarness.PM.proj.layers = [audioLayer({ name: 'Missing narration', dur: 4 })];

  await assert.rejects(
    missingHarness.PM.Audio.renderOffline(0, 4),
    /Audio media is missing for “Missing narration”/,
  );

  const brokenHarness = audioHarness({ decode: () => { throw new Error('decoder failed'); } });
  const broken = { ...audioAsset(decoded), audioBuffer: null, peaks: null, dur: 4 };
  brokenHarness.assets.set(broken.id, broken);
  brokenHarness.PM.proj.layers = [audioLayer({ name: 'Broken narration', dur: 4 })];

  await assert.rejects(
    brokenHarness.PM.Audio.renderOffline(0, 4),
    /Could not include “Broken narration” in the export/,
  );
});

test('offline export pins earlier decoded tracks while later tracks decode and preserves waveform peaks on eviction', async () => {
  const decoded = new FakeAudioBuffer([new Float32Array(16)], 16);
  const harness = audioHarness({ decodedBuffer: decoded });
  const huge = {
    numberOfChannels: 2,
    length: 50_000_000,
    sampleRate: 48_000,
    duration: 50_000_000 / 48_000,
    getChannelData() { return new Float32Array(0); },
  };
  const first = { ...audioAsset(decoded, 'asset-1'), audioBuffer: huge, dur: huge.duration, peaks: new Float32Array([.2, .8]) };
  const second = { ...audioAsset(decoded, 'asset-2'), audioBuffer: null, dur: decoded.duration, peaks: null };
  harness.assets.set(first.id, first);
  harness.assets.set(second.id, second);
  harness.PM.proj.layers = [
    audioLayer({ id: 'audio-1', dur: 1, d: { asset: first.id } }),
    audioLayer({ id: 'audio-2', dur: 1, d: { asset: second.id } }),
  ];

  await harness.PM.Audio.renderOffline(0, 1);

  const buffers = harness.offlineContexts[0].sources.map(source => source.buffer);
  assert.ok(buffers.includes(huge), 'the first buffer survives until its offline source owns it');
  assert.ok(buffers.includes(decoded), 'the later decoded track is also scheduled');
  assert.equal(first.audioBuffer, null, 'the large sample buffer may be evicted after export releases its pin');
  assert.deepEqual(Array.from(first.peaks, value => Number(value.toFixed(3))), [.2, .8], 'small waveform peaks survive decoded-sample eviction');
});

test('realtime export schedules the shared clip plan into an isolated audio stream and cleans it up', async () => {
  const buffer = new FakeAudioBuffer([new Float32Array(40)], 4);
  const harness = audioHarness();
  const asset = audioAsset(buffer);
  const layer = audioLayer({ from: 2, dur: 4, d: { trim: 1, fadeIn: .1, fadeOut: .2 } });
  harness.assets.set(asset.id, asset);
  harness.PM.proj.layers = [layer];

  const mix = await harness.PM.Audio.createRealtimeMix(1, 6);
  assert.ok(mix);
  assert.equal(mix.stream.getAudioTracks().length, 1);
  assert.equal(mix.start(.1), .1);
  const source = harness.contexts[0].sources[0];
  assert.deepEqual(source.starts[0], { when: 1.35, offset: 1, duration: 4 });
  assert.equal(source.connectedTo.connectedTo.stream, mix.stream, 'the mix is routed to the recorder stream, not the speaker master');
  mix.stop();
  assert.equal(source.stopCalls, 1);
  assert.equal(mix.stream.getAudioTracks()[0].stopped, true);
});
