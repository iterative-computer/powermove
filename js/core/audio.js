/* Powermove — decoded-buffer audio. One planner powers preview, waveforms, and export. */
(() => {
const PM = window.PM;

const AUDIO_EXTENSIONS = new Set(['wav', 'mp3', 'm4a', 'aac', 'ogg', 'oga', 'flac', 'aif', 'aiff']);
const MAX_DECODED_BYTES = 320 * 1024 * 1024;
const state = {
  context: null,
  master: null,
  voices: new Map(),
  running: false,
  generation: 0,
};

const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
const finite = (value, fallback = 0) => Number.isFinite(Number(value)) ? Number(value) : fallback;
const audioLayers = (project = PM.proj) => Array.isArray(project && project.layers)
  ? project.layers.filter(layer => layer && layer.type === 'audio') : [];

function accepts(file) {
  const mime = String(file && file.type || '').toLowerCase();
  if (mime.startsWith('audio/')) return true;
  const name = String(file && file.name || '');
  const extension = name.includes('.') ? name.split('.').pop().toLowerCase() : '';
  return AUDIO_EXTENSIONS.has(extension);
}

/* Runtime state is deliberately excluded from project JSON. This normalizer is
   also the one-way migration for audio layers saved by earlier implementations. */
function normalizeLayer(layer) {
  if (!layer || layer.type !== 'audio') return layer;
  const data = layer.d && typeof layer.d === 'object' ? layer.d : {};
  layer.d = {
    asset: typeof data.asset === 'string' && data.asset ? data.asset : null,
    trim: Math.max(0, finite(data.trim)),
    gain: clamp(finite(data.gain, 1), 0, 4),
    fadeIn: Math.max(0, finite(data.fadeIn)),
    fadeOut: Math.max(0, finite(data.fadeOut)),
  };
  layer.p = {};
  layer.fx = [];
  layer.masks = [];
  layer.parent = null;
  layer.blend = 'normal';
  layer.mblur = false;
  return layer;
}

function context() {
  if (state.context) return state.context;
  const Ctor = window.AudioContext || window.webkitAudioContext;
  if (!Ctor) throw new Error('Audio playback is not supported on this system');
  try { state.context = new Ctor({ latencyHint: 'interactive' }); }
  catch (error) { state.context = new Ctor(); }
  state.master = state.context.createGain();
  state.master.gain.value = 1;
  state.master.connect(state.context.destination);
  return state.context;
}

function decodeArrayBuffer(ctx, bytes) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (fn, value) => {
      if (settled) return;
      settled = true;
      fn(value);
    };
    try {
      const pending = ctx.decodeAudioData(bytes, value => finish(resolve, value), error => finish(reject, error));
      if (pending && typeof pending.then === 'function') pending.then(
        value => finish(resolve, value), error => finish(reject, error));
    } catch (error) { finish(reject, error); }
  });
}

function peakEnvelope(buffer, buckets = 640) {
  const length = Math.max(1, Math.min(buckets, buffer.length || buckets));
  const peaks = new Float32Array(length);
  const channels = Math.max(1, buffer.numberOfChannels || 1);
  const stride = Math.max(1, Math.floor((buffer.length || 1) / length));
  for (let bucket = 0; bucket < length; bucket++) {
    const start = bucket * stride;
    const end = bucket === length - 1 ? buffer.length : Math.min(buffer.length, start + stride);
    let peak = 0;
    for (let channel = 0; channel < channels; channel++) {
      const samples = buffer.getChannelData(channel);
      for (let index = start; index < end; index++) peak = Math.max(peak, Math.abs(samples[index] || 0));
    }
    peaks[bucket] = peak;
  }
  return peaks;
}

function decodedBytes(buffer) {
  return Math.max(0, finite(buffer && buffer.length) * Math.max(1, finite(buffer && buffer.numberOfChannels, 1)) * 4);
}

function activeAssetIds() {
  return new Set([...state.voices.values()].map(voice => voice.assetId));
}

function trimDecodedCache() {
  if (!PM.assets || !PM.assets.map) return;
  const decoded = [...PM.assets.map.values()].filter(asset => asset && asset.kind === 'audio' && asset.audioBuffer);
  let total = decoded.reduce((sum, asset) => sum + decodedBytes(asset.audioBuffer), 0);
  if (total <= MAX_DECODED_BYTES || decoded.length < 2) return;
  const active = activeAssetIds();
  decoded.sort((a, b) => finite(a.audioUsedAt) - finite(b.audioUsedAt));
  for (const asset of decoded) {
    if (total <= MAX_DECODED_BYTES) break;
    if (active.has(asset.id) || asset.audioDecoding) continue;
    total -= decodedBytes(asset.audioBuffer);
    asset.audioBuffer = null;
    asset.peaks = null;
  }
}

async function decodeAsset(asset) {
  if (!asset || asset.kind !== 'audio') throw new Error('Audio asset is unavailable');
  if (asset.audioBuffer) {
    asset.audioUsedAt = Date.now();
    return asset.audioBuffer;
  }
  if (asset.audioDecoding) return asset.audioDecoding;
  if (!asset.audioBlob) throw new Error('Audio media is missing · import it again to relink it');
  const token = asset.audioToken;
  asset.audioDecoding = (async () => {
    try {
      const bytes = await asset.audioBlob.arrayBuffer();
      const buffer = await decodeArrayBuffer(context(), bytes.slice(0));
      if (!buffer || !(buffer.duration > 0)) throw new Error('The decoder returned an empty audio file');
      if (asset.audioDisposed || asset.audioToken !== token) return null;
      asset.audioBuffer = buffer;
      asset.dur = buffer.duration;
      asset.channels = buffer.numberOfChannels || 1;
      asset.sampleRate = buffer.sampleRate || 0;
      asset.peaks = peakEnvelope(buffer);
      asset.audioUsedAt = Date.now();
      trimDecodedCache();
      PM.bus && PM.bus.emit('audio:decoded', asset.id);
      PM.invalidate && PM.invalidate('timeline');
      return buffer;
    } catch (error) {
      if (error && /missing|decoder|supported/i.test(error.message || '')) throw error;
      throw new Error('Could not decode this audio file · try WAV, MP3, M4A, AAC, OGG, FLAC, or AIFF');
    } finally {
      asset.audioDecoding = null;
    }
  })();
  return asset.audioDecoding;
}

async function prepareAsset({ id, name, blob, meta = {} }) {
  const asset = {
    id,
    name,
    kind: 'audio',
    url: null,
    el: null,
    w: 0,
    h: 0,
    dur: Math.max(0, finite(meta.dur)),
    size: finite(blob && blob.size, finite(meta.size)),
    channels: Math.max(0, finite(meta.channels)),
    sampleRate: Math.max(0, finite(meta.sampleRate)),
    audioBlob: blob,
    audioBuffer: null,
    audioDecoding: null,
    audioDisposed: false,
    audioToken: Symbol('audio-asset'),
    peaks: null,
  };
  /* Imports are decoded before they can mutate project source. Restores already
     have trusted duration metadata and stay lazy until playback/waveform demand. */
  if (!(asset.dur > 0)) await decodeAsset(asset);
  return asset;
}

function disposeAsset(asset) {
  if (!asset || asset.kind !== 'audio') return;
  asset.audioDisposed = true;
  asset.audioToken = Symbol('disposed-audio-asset');
  for (const [id, voice] of state.voices) if (voice.assetId === asset.id) stopVoice(id);
  asset.audioBuffer = null;
  asset.peaks = null;
  asset.audioBlob = null;
}

function gainAt(layer, localTime) {
  const data = layer && layer.d || {};
  const base = clamp(finite(data.gain, 1), 0, 4);
  const duration = Math.max(0, finite(layer && layer.dur));
  const local = clamp(finite(localTime), 0, duration);
  const fadeIn = Math.max(0, finite(data.fadeIn));
  const fadeOut = Math.max(0, finite(data.fadeOut));
  const entering = fadeIn > 0 ? clamp(local / fadeIn, 0, 1) : 1;
  const leaving = fadeOut > 0 ? clamp((duration - local) / fadeOut, 0, 1) : 1;
  return base * Math.min(entering, leaving);
}

function envelope(layer, localStart, duration, samples = 96) {
  const count = Math.max(2, Math.floor(samples));
  const values = new Float32Array(count);
  for (let index = 0; index < count; index++) {
    values[index] = gainAt(layer, localStart + duration * index / (count - 1));
  }
  return values;
}

function soloActive(project = PM.proj) {
  return !!(project && Array.isArray(project.layers) && project.layers.some(layer => layer && layer.on !== false && layer.solo));
}

function plan(layer, asset, rangeStart, rangeEnd, options = {}) {
  if (!layer || layer.type !== 'audio' || layer.on === false || !layer.d || !layer.d.asset || !asset) return null;
  if (options.solo && !layer.solo) return null;
  const clipStart = Math.max(0, finite(layer.from));
  const clipEnd = clipStart + Math.max(0, finite(layer.dur));
  const start = Math.max(clipStart, finite(rangeStart));
  const end = Math.min(clipEnd, finite(rangeEnd, clipEnd));
  if (end - start <= 1e-4) return null;
  const localStart = start - clipStart;
  const sourceOffset = Math.max(0, finite(layer.d.trim) + localStart);
  const sourceDuration = asset.audioBuffer ? asset.audioBuffer.duration : finite(asset.dur);
  if (!(sourceDuration > 0) || sourceOffset >= sourceDuration - 1e-4) return null;
  const duration = Math.min(end - start, sourceDuration - sourceOffset);
  if (duration <= 1e-4) return null;
  return { layer, asset, start, end: start + duration, duration, localStart, sourceOffset };
}

function voiceSignature(layer, asset) {
  const data = layer.d || {};
  return [asset.id, asset.audioToken && String(asset.audioToken), finite(layer.from), finite(layer.dur),
    finite(data.trim), finite(data.gain, 1), finite(data.fadeIn), finite(data.fadeOut), !!layer.solo].join('|');
}

function publishState() {
  if (typeof document === 'undefined' || !document.documentElement) return;
  const root = document.documentElement;
  root.dataset.audioEngine = 'decoded-buffer-v2';
  root.dataset.audioState = state.running ? 'playing' : 'paused';
  root.dataset.audioVoices = String(state.voices.size);
  const first = state.voices.values().next().value;
  if (first) root.dataset.audioSourceOffset = String(Math.round(first.sourceOffset * 1000) / 1000);
  else delete root.dataset.audioSourceOffset;
}

function stopVoice(id) {
  const voice = state.voices.get(id);
  if (!voice) return;
  state.voices.delete(id);
  voice.ended = true;
  try { voice.source.onended = null; voice.source.stop(); } catch (error) { }
  try { voice.source.disconnect(); } catch (error) { }
  try { voice.gain.disconnect(); } catch (error) { }
  publishState();
}

function stopAll() {
  for (const id of [...state.voices.keys()]) stopVoice(id);
  publishState();
}

function applyEnvelope(param, layer, localStart, duration, at) {
  const curve = envelope(layer, localStart, duration);
  try {
    param.cancelScheduledValues(at);
    if (duration > .002 && typeof param.setValueCurveAtTime === 'function') param.setValueCurveAtTime(curve, at, duration);
    else param.setValueAtTime(curve[0], at);
  } catch (error) { param.value = curve[0]; }
  return curve;
}

function startVoice(clip, transportTime) {
  const ctx = context();
  const source = ctx.createBufferSource();
  const gain = ctx.createGain();
  source.buffer = clip.asset.audioBuffer;
  source.connect(gain).connect(state.master);
  /* A clip whose in-point is later than the current playhead must be queued on
     the audio clock, not started early. Once the playhead is inside the clip,
     plan() sets clip.start to transportTime and this delay naturally becomes 0. */
  const at = ctx.currentTime + Math.max(0, clip.start - finite(transportTime));
  const curve = applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at);
  const voice = {
    source,
    gain,
    layerId: clip.layer.id,
    assetId: clip.asset.id,
    signature: voiceSignature(clip.layer, clip.asset),
    sourceOffset: clip.sourceOffset,
    duration: clip.duration,
    curve,
    ended: false,
  };
  state.voices.set(clip.layer.id, voice);
  source.onended = () => {
    if (state.voices.get(clip.layer.id) !== voice) return;
    state.voices.delete(clip.layer.id);
    try { source.disconnect(); } catch (error) { }
    try { gain.disconnect(); } catch (error) { }
    publishState();
  };
  try { source.start(at, clip.sourceOffset, clip.duration); }
  catch (error) { stopVoice(clip.layer.id); return null; }
  clip.asset.audioUsedAt = Date.now();
  publishState();
  return voice;
}

function requestDecode(layer, asset, generation) {
  decodeAsset(asset).then(buffer => {
    if (!buffer || !state.running || generation !== state.generation) return;
    const current = PM.proj && PM.proj.layers && PM.proj.layers.find(item => item.id === layer.id);
    if (!current || current.d.asset !== asset.id || !PM.assets || PM.assets.get(asset.id) !== asset) return;
    sync(PM.time, true);
  }).catch(error => {
    if (generation === state.generation && PM.toast) PM.toast(error.message || 'Could not decode audio', 5000);
  });
}

function sync(time, force = false) {
  if (!state.running || !PM.proj) { stopAll(); return; }
  const generation = state.generation;
  const desired = new Set();
  const solo = soloActive(PM.proj);
  for (const layer of audioLayers()) {
    const asset = layer.d && layer.d.asset && PM.assets && PM.assets.get(layer.d.asset);
    const clip = plan(layer, asset, time, layer.from + layer.dur, { solo });
    if (!clip) continue;
    desired.add(layer.id);
    if (!asset.audioBuffer) {
      stopVoice(layer.id);
      requestDecode(layer, asset, generation);
      continue;
    }
    const voice = state.voices.get(layer.id);
    const signature = voiceSignature(layer, asset);
    if (force || !voice || voice.signature !== signature) {
      stopVoice(layer.id);
      startVoice(clip, time);
    }
  }
  for (const id of [...state.voices.keys()]) if (!desired.has(id)) stopVoice(id);
  publishState();
}

function start(time) {
  state.running = true;
  const generation = ++state.generation;
  let ctx;
  try { ctx = context(); }
  catch (error) { state.running = false; publishState(); PM.toast && PM.toast(error.message, 5000); return; }
  sync(time, true);
  if (ctx.state === 'suspended' && typeof ctx.resume === 'function') {
    Promise.resolve(ctx.resume()).then(() => {
      if (state.running && generation === state.generation) sync(PM.time, true);
    }).catch(() => {});
  }
}

function pause() {
  state.running = false;
  state.generation++;
  stopAll();
}

function seek(time) {
  if (!state.running) return;
  state.generation++;
  sync(time, true);
}

function tick(time) { sync(time, false); }

function reconcile() {
  if (!PM.playing) { pause(); return; }
  state.generation++;
  sync(PM.time, true);
}

function drawWaveform(ctx, layer, options = {}) {
  const x = finite(options.x);
  const y = finite(options.y);
  const width = Math.max(0, finite(options.width, finite(options.w)));
  const height = Math.max(0, finite(options.height, finite(options.h)));
  const clipLeft = finite(options.clipLeft, x);
  const asset = layer && layer.d && PM.assets && PM.assets.get(layer.d.asset);
  const left = Math.max(x, clipLeft);
  const right = x + width;
  if (!(right > left && height > 2)) return false;
  if (!asset || !asset.peaks || !asset.peaks.length) {
    if (asset && asset.kind === 'audio') decodeAsset(asset).catch(() => {});
    ctx.fillStyle = options.placeholderColor || 'rgba(255,255,255,.22)';
    ctx.fillRect(left, y + height / 2, right - left, 1);
    return false;
  }
  const peaks = asset.peaks;
  const duration = Math.max(.001, finite(asset.dur));
  const trim = Math.max(0, finite(layer.d.trim));
  const step = Math.max(2, finite(options.step, 2.5));
  ctx.fillStyle = options.color || 'rgba(255,255,255,.52)';
  for (let px = left; px < right; px += step) {
    const local = (px - x) / Math.max(1, width) * Math.max(0, finite(layer.dur));
    const sourceRatio = clamp((trim + local) / duration, 0, 1);
    const peak = peaks[Math.min(peaks.length - 1, Math.floor(sourceRatio * peaks.length))] || 0;
    const bar = Math.max(1, peak * (height - 4));
    ctx.fillRect(px, y + (height - bar) / 2, Math.max(1, step - 1), bar);
  }
  return true;
}

async function renderOffline(t0, t1) {
  const from = Math.max(0, finite(t0));
  const to = Math.max(from, finite(t1, from));
  if (to - from <= 1e-4 || !PM.proj) return null;
  const solo = soloActive(PM.proj);
  const clips = [];
  for (const layer of audioLayers()) {
    const asset = layer.d && layer.d.asset && PM.assets && PM.assets.get(layer.d.asset);
    if (!asset) continue;
    try { await decodeAsset(asset); } catch (error) { console.warn('Audio layer skipped during export', error); continue; }
    const clip = plan(layer, asset, from, to, { solo });
    if (clip) clips.push(clip);
  }
  if (!clips.length) return null;
  const Offline = window.OfflineAudioContext;
  if (!Offline) return null;
  const sampleRate = 48000;
  let offline;
  try { offline = new Offline(2, Math.ceil((to - from) * sampleRate), sampleRate); }
  catch (error) { return null; }
  for (const clip of clips) {
    try {
      const source = offline.createBufferSource();
      const gain = offline.createGain();
      source.buffer = clip.asset.audioBuffer;
      source.connect(gain).connect(offline.destination);
      const at = clip.start - from;
      applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at);
      source.start(at, clip.sourceOffset, clip.duration);
    } catch (error) { console.warn('Audio layer skipped during export', error); }
  }
  try { return await offline.startRendering(); }
  catch (error) { console.warn('Audio mixdown failed', error); return null; }
}

function opusHead(channels, sampleRate) {
  const out = new Uint8Array(19);
  out.set([79, 112, 117, 115, 72, 101, 97, 100], 0); // OpusHead
  out[8] = 1;
  out[9] = channels;
  out[10] = 56; out[11] = 1; // 312-sample pre-skip, little endian
  new DataView(out.buffer).setUint32(12, sampleRate, true);
  out[18] = 0;
  return out;
}

async function encodeOpus(buffer) {
  const Encoder = window.AudioEncoder;
  const Data = window.AudioData;
  if (!buffer || !Encoder || !Data) return null;
  const channels = Math.min(2, Math.max(1, buffer.numberOfChannels || 1));
  const config = { codec: 'opus', sampleRate: buffer.sampleRate, numberOfChannels: channels, bitrate: 160000 };
  const support = await Encoder.isConfigSupported(config).catch(() => null);
  if (!support || !support.supported) return null;
  const chunks = [];
  let privateData = null;
  let encodeError = null;
  const encoder = new Encoder({
    output(chunk, metadata) {
      const description = metadata && metadata.decoderConfig && metadata.decoderConfig.description;
      if (!privateData && description) privateData = new Uint8Array(description);
      const bytes = new Uint8Array(chunk.byteLength);
      chunk.copyTo(bytes);
      chunks.push({ ts: Number(chunk.timestamp), data: bytes });
    },
    error(error) { encodeError = error; },
  });
  encoder.configure(config);
  const source = Array.from({ length: channels }, (_, channel) => buffer.getChannelData(channel));
  const frameLength = Math.max(1, Math.floor(buffer.sampleRate * .02));
  for (let offset = 0; offset < buffer.length; offset += frameLength) {
    const frames = Math.min(frameLength, buffer.length - offset);
    const planar = new Float32Array(frames * channels);
    for (let channel = 0; channel < channels; channel++) planar.set(source[channel].subarray(offset, offset + frames), channel * frames);
    const data = new Data({
      format: 'f32-planar',
      sampleRate: buffer.sampleRate,
      numberOfFrames: frames,
      numberOfChannels: channels,
      timestamp: Math.round(offset / buffer.sampleRate * 1e6),
      data: planar,
    });
    encoder.encode(data);
    data.close();
    if (encoder.encodeQueueSize > 16) await new Promise(resolve => setTimeout(resolve, 4));
  }
  await encoder.flush();
  encoder.close();
  if (encodeError || !chunks.length) return null;
  return { chunks, priv: privateData || opusHead(channels, buffer.sampleRate), rate: buffer.sampleRate, channels };
}

function hasAudibleLayers(project = PM.proj) {
  return audioLayers(project).some(layer => layer.on !== false && layer.d && layer.d.asset);
}

const Audio = PM.Audio = {
  accepts,
  normalizeLayer,
  prepareAsset,
  disposeAsset,
  decodeAsset,
  plan,
  gainAt,
  envelope,
  start,
  pause,
  seek,
  tick,
  reconcile,
  drawWaveform,
  renderOffline,
  encodeOpus,
  hasAudibleLayers,
  inspect: () => ({
    engine: 'decoded-buffer-v2',
    running: state.running,
    generation: state.generation,
    voices: [...state.voices.values()].map(voice => ({
      layerId: voice.layerId,
      assetId: voice.assetId,
      sourceOffset: voice.sourceOffset,
      duration: voice.duration,
    })),
    contextState: state.context && state.context.state || 'uninitialized',
  }),
};

PM.bus.on('layers', reconcile);
PM.bus.on('assets', reconcile);
PM.bus.on('project', () => { pause(); if (PM.playing) start(PM.time); });
publishState();
})();
