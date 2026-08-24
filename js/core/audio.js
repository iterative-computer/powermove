/* Powermove — decoded-buffer audio. One planner powers preview, waveforms, and export. */
(() => {
const PM = window.PM;

const AUDIO_EXTENSIONS = new Set(['wav', 'mp3', 'm4a', 'aac', 'ogg', 'oga', 'flac', 'aif', 'aiff']);
const MAX_DECODED_BYTES = 320 * 1024 * 1024;
const MAX_PRECOMP_DEPTH = 8;
const LIVE_LOOKAHEAD = 1;
const DRIFT_TOLERANCE = .075;
const DECODE_RETRY_MS = 5000;
const state = {
  context: null,
  master: null,
  voices: new Map(),
  decodeRequests: new Map(),
  pinnedAssets: new Set(),
  project: null,
  running: false,
  generation: 0,
};

const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
const finite = (value, fallback = 0) => Number.isFinite(Number(value)) ? Number(value) : fallback;
/* Flatten audio through the same precomp time mapping used by the compositor.
   A lightweight wrapper keeps the original source/fade timing while recording
   every ancestor's visible window. Path-based IDs let one nested comp be used
   more than once without its voices colliding. */
function audioLayers(project = PM.proj) {
  if (!project || !Array.isArray(project.layers)) return [];
  const output = [];
  const root = project;
  const visit = (comp, offset, windowStart, windowEnd, path, depth) => {
    if (!comp || !Array.isArray(comp.layers) || depth > MAX_PRECOMP_DEPTH) return;
    const solo = comp.layers.some(layer => layer && layer.solo);
    for (const layer of comp.layers) {
      if (!layer || layer.on === false || (solo && !layer.solo)) continue;
      const naturalStart = offset + Math.max(0, finite(layer.from));
      const start = Math.max(windowStart, naturalStart);
      const end = Math.min(windowEnd, naturalStart + Math.max(0, finite(layer.dur)));
      if (end - start <= 1e-4) continue;
      const itemPath = path ? `${path}/${layer.id}` : layer.id;
      if (layer.type === 'audio') {
        output.push({
          ...layer,
          id: itemPath,
          from: naturalStart,
          _audioSourceId: layer.id,
          _audioWindowStart: start,
          _audioWindowEnd: end,
        });
        continue;
      }
      if (layer.type !== 'precomp' || !layer.d || !layer.d.comp) continue;
      const nested = root.comps && root.comps[layer.d.comp];
      visit(nested, naturalStart, start, end, itemPath, depth + 1);
    }
  };
  visit(project, 0, 0, Infinity, '', 0);
  return output;
}

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
  return new Set([
    ...state.pinnedAssets,
    ...[...state.voices.values()].map(voice => voice.assetId),
  ]);
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
    /* Peak envelopes are tiny and remain useful after decoded samples are
       evicted. Keeping them prevents a visible timeline from decoding the same
       large file again just to repaint its waveform. */
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
  if (asset.audioRetryAt > Date.now() && asset.audioDecodeError) throw asset.audioDecodeError;
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
      asset.audioDecodeError = null;
      asset.audioRetryAt = 0;
      trimDecodedCache();
      PM.bus && PM.bus.emit('audio:decoded', asset.id);
      PM.invalidate && PM.invalidate('timeline');
      return buffer;
    } catch (error) {
      const failure = error && /missing|decoder|supported/i.test(error.message || '')
        ? error
        : new Error('Could not decode this audio file · try WAV, MP3, M4A, AAC, OGG, FLAC, or AIFF');
      asset.audioDecodeError = failure;
      asset.audioRetryAt = Date.now() + DECODE_RETRY_MS;
      throw failure;
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
    audioDecodeError: null,
    audioRetryAt: 0,
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
  for (const [id, request] of state.decodeRequests) if (request.asset === asset) state.decodeRequests.delete(id);
}

function gainAt(layer, localTime, audibleDuration) {
  const data = layer && layer.d || {};
  const base = clamp(finite(data.gain, 1), 0, 4);
  const duration = Math.max(0, finite(audibleDuration, finite(layer && layer.dur)));
  const local = clamp(finite(localTime), 0, duration);
  const fadeIn = Math.max(0, finite(data.fadeIn));
  const fadeOut = Math.max(0, finite(data.fadeOut));
  const entering = fadeIn > 0 ? clamp(local / fadeIn, 0, 1) : 1;
  const leaving = fadeOut > 0 ? clamp((duration - local) / fadeOut, 0, 1) : 1;
  return base * Math.min(entering, leaving);
}

function envelope(layer, localStart, duration, samples = 96, audibleDuration) {
  const count = Math.max(2, Math.floor(samples));
  const values = new Float32Array(count);
  for (let index = 0; index < count; index++) {
    values[index] = gainAt(layer, localStart + duration * index / (count - 1), audibleDuration);
  }
  return values;
}

/* gainAt() is piecewise linear. Scheduling only the exact slope changes keeps a
   100 ms fade at 100 ms even when the clip lasts for minutes. */
function envelopePoints(layer, localStart, duration, audibleDuration) {
  const data = layer && layer.d || {};
  const full = Math.max(0, finite(audibleDuration, finite(layer && layer.dur)));
  const start = clamp(finite(localStart), 0, full);
  const end = clamp(start + Math.max(0, finite(duration)), start, full);
  const fadeIn = Math.max(0, finite(data.fadeIn));
  const fadeOut = Math.max(0, finite(data.fadeOut));
  const times = [start, end];
  if (fadeIn > start + 1e-6 && fadeIn < end - 1e-6) times.push(fadeIn);
  const fadeOutStart = full - fadeOut;
  if (fadeOut > 0 && fadeOutStart > start + 1e-6 && fadeOutStart < end - 1e-6) times.push(fadeOutStart);
  if (fadeIn > 0 && fadeOut > 0 && fadeIn > fadeOutStart) {
    const crossing = full * fadeIn / (fadeIn + fadeOut);
    if (crossing > start + 1e-6 && crossing < end - 1e-6) times.push(crossing);
  }
  return [...new Set(times)].sort((a, b) => a - b).map(local => ({
    local,
    value: gainAt(layer, local, full),
  }));
}

function plan(layer, asset, rangeStart, rangeEnd, options = {}) {
  if (!layer || layer.type !== 'audio' || layer.on === false || !layer.d || !layer.d.asset || !asset) return null;
  if (options.solo && !layer.solo) return null;
  const layerStart = Math.max(0, finite(layer.from));
  const layerDuration = Math.max(0, finite(layer.dur));
  const windowStart = Math.max(layerStart, finite(layer._audioWindowStart, layerStart));
  const windowEnd = Math.min(layerStart + layerDuration, finite(layer._audioWindowEnd, layerStart + layerDuration));
  const sourceDuration = asset.audioBuffer ? asset.audioBuffer.duration : finite(asset.dur);
  const trim = Math.max(0, finite(layer.d.trim));
  const audibleDuration = Math.min(layerDuration, Math.max(0, sourceDuration - trim));
  const clipStart = windowStart;
  const clipEnd = Math.min(windowEnd, layerStart + audibleDuration);
  const start = Math.max(clipStart, finite(rangeStart));
  const end = Math.min(clipEnd, finite(rangeEnd, clipEnd));
  if (end - start <= 1e-4) return null;
  const localStart = start - layerStart;
  const sourceOffset = trim + localStart;
  if (!(sourceDuration > 0) || sourceOffset >= sourceDuration - 1e-4) return null;
  const duration = Math.min(end - start, sourceDuration - sourceOffset);
  if (duration <= 1e-4) return null;
  return { layer, asset, start, end: start + duration, duration, localStart, sourceOffset, audibleDuration };
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

function applyEnvelope(param, layer, localStart, duration, at, audibleDuration) {
  const points = envelopePoints(layer, localStart, duration, audibleDuration);
  try {
    param.cancelScheduledValues(at);
    param.setValueAtTime(points[0].value, at);
    if (duration > .002 && typeof param.linearRampToValueAtTime === 'function') {
      for (let index = 1; index < points.length; index++) {
        param.linearRampToValueAtTime(points[index].value, at + points[index].local - localStart);
      }
    } else if (duration > .002 && typeof param.setValueCurveAtTime === 'function') {
      param.setValueCurveAtTime(envelope(layer, localStart, duration, 256, audibleDuration), at, duration);
    }
  } catch (error) { param.value = points[0].value; }
  return points;
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
  const curve = applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at, clip.audibleDuration);
  const voice = {
    source,
    gain,
    layerId: clip.layer.id,
    assetId: clip.asset.id,
    signature: voiceSignature(clip.layer, clip.asset),
    sourceOffset: clip.sourceOffset,
    duration: clip.duration,
    curve,
    scheduledAt: at,
    transportStart: clip.start,
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
  if (asset.audioRetryAt > Date.now()) return;
  const existing = state.decodeRequests.get(layer.id);
  if (existing && existing.asset === asset && existing.generation === generation) return;
  const request = { asset, generation };
  state.decodeRequests.set(layer.id, request);
  decodeAsset(asset).then(buffer => {
    if (!buffer || !state.running || generation !== state.generation) return;
    const current = audioLayers().find(item => item.id === layer.id);
    if (!current || current.d.asset !== asset.id || !PM.assets || PM.assets.get(asset.id) !== asset) return;
    sync(PM.time, true);
  }).catch(error => {
    if (generation === state.generation && PM.toast) PM.toast(error.message || 'Could not decode audio', 5000);
  }).finally(() => {
    if (state.decodeRequests.get(layer.id) === request) state.decodeRequests.delete(layer.id);
  });
}

function voiceDrift(voice, layer, time, ctx) {
  if (!voice || !ctx) return Infinity;
  const audioUntilStart = voice.scheduledAt - ctx.currentTime;
  const transportUntilStart = voice.transportStart - time;
  if (audioUntilStart > 0 || transportUntilStart > 0) {
    return Math.abs(audioUntilStart - transportUntilStart);
  }
  const audioSourceTime = voice.sourceOffset + Math.max(0, ctx.currentTime - voice.scheduledAt);
  const transportSourceTime = Math.max(0, finite(layer.d && layer.d.trim) + time - finite(layer.from));
  return Math.abs(audioSourceTime - transportSourceTime);
}

function sync(time, force = false) {
  if (!state.running || !PM.proj) { stopAll(); return; }
  const generation = state.generation;
  const desired = new Set();
  for (const layer of audioLayers()) {
    const audibleStart = Math.max(finite(layer.from), finite(layer._audioWindowStart, finite(layer.from)));
    if (audibleStart - time > LIVE_LOOKAHEAD) continue;
    const asset = layer.d && layer.d.asset && PM.assets && PM.assets.get(layer.d.asset);
    const clip = plan(layer, asset, time, layer.from + layer.dur);
    if (!clip) continue;
    desired.add(layer.id);
    if (!asset.audioBuffer) {
      stopVoice(layer.id);
      requestDecode(layer, asset, generation);
      continue;
    }
    const voice = state.voices.get(layer.id);
    const signature = voiceSignature(layer, asset);
    const drifted = voice && voiceDrift(voice, layer, time, state.context) > DRIFT_TOLERANCE;
    if (force || !voice || voice.signature !== signature || drifted) {
      stopVoice(layer.id);
      startVoice(clip, time);
    }
  }
  for (const id of [...state.voices.keys()]) if (!desired.has(id)) stopVoice(id);
  publishState();
}

function start(time) {
  state.running = true;
  state.project = PM.proj || null;
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
  state.project = PM.proj || null;
  if (!PM.playing) { pause(); return; }
  state.generation++;
  sync(PM.time, true);
}

function reconcileProject() {
  const changed = state.project !== PM.proj;
  state.project = PM.proj || null;
  if (!changed) return;
  reconcile();
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
    if (asset && asset.kind === 'audio' && !asset.audioDecoding && !(asset.audioRetryAt > Date.now())) {
      decodeAsset(asset).catch(() => {});
    }
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

async function collectDecodedClips(t0, t1) {
  const from = Math.max(0, finite(t0));
  const to = Math.max(from, finite(t1, from));
  if (to - from <= 1e-4 || !PM.proj) return { from, to, clips: [], pinned: new Set() };
  const clips = [];
  const pinned = new Set();
  for (const layer of audioLayers()) {
    const asset = layer.d && layer.d.asset && PM.assets && PM.assets.get(layer.d.asset);
    if (!asset) continue;
    state.pinnedAssets.add(asset.id);
    pinned.add(asset.id);
    try { await decodeAsset(asset); }
    catch (error) { console.warn('Audio layer skipped during export', error); continue; }
    const clip = plan(layer, asset, from, to);
    if (clip) clips.push(clip);
  }
  return { from, to, clips, pinned };
}

function releasePins(pinned) {
  for (const id of pinned || []) state.pinnedAssets.delete(id);
  trimDecodedCache();
}

async function renderOffline(t0, t1) {
  const prepared = await collectDecodedClips(t0, t1);
  try {
    const { from, to, clips } = prepared;
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
        applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at, clip.audibleDuration);
        source.start(at, clip.sourceOffset, clip.duration);
      } catch (error) { console.warn('Audio layer skipped during export', error); }
    }
    try { return await offline.startRendering(); }
    catch (error) { console.warn('Audio mixdown failed', error); return null; }
  } finally {
    releasePins(prepared.pinned);
  }
}

/* MediaRecorder needs a real audio MediaStream. This path schedules the same
   decoded clips and exact envelope points as preview/offline export, but routes
   them to an isolated stream destination instead of the speakers. */
async function createRealtimeMix(t0, t1) {
  const prepared = await collectDecodedClips(t0, t1);
  try {
    const { from, clips } = prepared;
    if (!clips.length) return null;
    const ctx = context();
    if (typeof ctx.createMediaStreamDestination !== 'function') {
      throw new Error('Realtime audio export is not supported on this system');
    }
    if (ctx.state === 'suspended' && typeof ctx.resume === 'function') await ctx.resume();
    const destination = ctx.createMediaStreamDestination();
    const nodes = [];
    for (const clip of clips) {
      const source = ctx.createBufferSource();
      const gain = ctx.createGain();
      source.buffer = clip.asset.audioBuffer;
      source.connect(gain).connect(destination);
      nodes.push({ source, gain, clip });
    }
    let started = false;
    let stopped = false;
    return {
      stream: destination.stream,
      start(leadSeconds = .06) {
        if (started || stopped) return 0;
        started = true;
        const lead = Math.max(.02, finite(leadSeconds, .06));
        const base = ctx.currentTime + lead;
        for (const { source, gain, clip } of nodes) {
          const at = base + clip.start - from;
          applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at, clip.audibleDuration);
          source.start(at, clip.sourceOffset, clip.duration);
        }
        return lead;
      },
      stop() {
        if (stopped) return;
        stopped = true;
        for (const { source, gain } of nodes) {
          try { source.stop(); } catch (error) { }
          try { source.disconnect(); } catch (error) { }
          try { gain.disconnect(); } catch (error) { }
        }
        for (const track of destination.stream.getTracks ? destination.stream.getTracks() : []) {
          try { track.stop(); } catch (error) { }
        }
      },
    };
  } finally {
    releasePins(prepared.pinned);
  }
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

async function supportsOpus() {
  const Encoder = window.AudioEncoder;
  const Data = window.AudioData;
  if (!Encoder || !Data || typeof Encoder.isConfigSupported !== 'function') return false;
  const config = { codec: 'opus', sampleRate: 48000, numberOfChannels: 2, bitrate: 160000 };
  const support = await Encoder.isConfigSupported(config).catch(() => null);
  return !!(support && support.supported);
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
  envelopePoints,
  start,
  pause,
  seek,
  tick,
  reconcile,
  drawWaveform,
  renderOffline,
  createRealtimeMix,
  supportsOpus,
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
PM.bus.on('project', reconcileProject);
publishState();
})();
