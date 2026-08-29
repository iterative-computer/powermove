/* Ported from js/core/audio.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const AUDIO_EXTENSIONS: any = new Set(['wav', 'mp3', 'm4a', 'aac', 'ogg', 'oga', 'flac', 'aif', 'aiff']);
const MAX_DECODED_BYTES: any = 320 * 1024 * 1024;
const MAX_PRECOMP_DEPTH: any = 8;
const LIVE_LOOKAHEAD: any = 1;
const DRIFT_TOLERANCE: any = .075;
const DECODE_RETRY_MS: any = 5000;
const state: any = {
  context: null,
  master: null,
  voices: new Map(),
  decodeRequests: new Map(),
  pinnedAssets: new Set(),
  project: null,
  running: false,
  generation: 0,
};

const clamp: any = (value: any, min: any, max: any) => Math.max(min, Math.min(max, value));
const finite: any = (value: any, fallback: any = 0) => Number.isFinite(Number(value)) ? Number(value) : fallback;
/* Flatten audio through the same precomp time mapping used by the compositor.
   A lightweight wrapper keeps the original source/fade timing while recording
   every ancestor's visible window. Path-based IDs let one nested comp be used
   more than once without its voices colliding. */
function audioLayers(project: any = PM.proj) {
  if (!project || !Array.isArray(project.layers)) return [];
  const output: any = [];
  const root: any = project;
  const visit: any = (comp: any, offset: any, windowStart: any, windowEnd: any, path: any, depth: any) => {
    if (!comp || !Array.isArray(comp.layers) || depth > MAX_PRECOMP_DEPTH) return;
    for (const layer of comp.layers) {
      if (!layer || layer.on === false) continue;
      const naturalStart: any = offset + Math.max(0, finite(layer.from));
      const start: any = Math.max(windowStart, naturalStart);
      const end: any = Math.min(windowEnd, naturalStart + Math.max(0, finite(layer.dur)));
      if (end - start <= 1e-4) continue;
      const itemPath: any = path ? `${path}/${layer.id}` : layer.id;
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
      const nested: any = root.comps && root.comps[layer.d.comp];
      visit(nested, naturalStart, start, end, itemPath, depth + 1);
    }
  };
  visit(project, 0, 0, Infinity, '', 0);
  return output;
}

function accepts(file: any) {
  const mime: any = String(file && file.type || '').toLowerCase();
  if (mime.startsWith('audio/')) return true;
  const name: any = String(file && file.name || '');
  const extension: any = name.includes('.') ? name.split('.').pop().toLowerCase() : '';
  return AUDIO_EXTENSIONS.has(extension);
}

/* Runtime state is deliberately excluded from project JSON. This normalizer is
   also the one-way migration for audio layers saved by earlier implementations. */
function normalizeLayer(layer: any) {
  if (!layer || layer.type !== 'audio') return layer;
  const data: any = layer.d && typeof layer.d === 'object' ? layer.d : {};
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
  const Ctor: any = window.AudioContext || (window as any).webkitAudioContext;
  if (!Ctor) throw new Error('Audio playback is not supported on this system');
  try { state.context = new Ctor({ latencyHint: 'interactive' }); }
  catch (error: any) { state.context = new Ctor(); }
  state.master = state.context.createGain();
  state.master.gain.value = 1;
  state.master.connect(state.context.destination);
  return state.context;
}

function decodeArrayBuffer(ctx: any, bytes: any) {
  return new Promise((resolve: any, reject: any) => {
    let settled: any = false;
    const finish: any = (fn: any, value: any) => {
      if (settled) return;
      settled = true;
      fn(value);
    };
    try {
      const pending: any = ctx.decodeAudioData(bytes, (value: any) => finish(resolve, value), (error: any) => finish(reject, error));
      if (pending && typeof pending.then === 'function') pending.then(
        (value: any) => finish(resolve, value), (error: any) => finish(reject, error));
    } catch (error: any) { finish(reject, error); }
  });
}

/* Waveform peaks at a fixed source rate, never a fixed count: a bucket must
   span a few milliseconds so the envelope keeps its dynamics. Squashing a
   whole file into a few hundred max-abs buckets flattens every bucket to the
   file's loudest sample and draws a solid wall. */
const PEAKS_PER_SECOND: any = 100;
function peakEnvelope(buffer: any, rate: any = PEAKS_PER_SECOND) {
  const frames: any = Math.max(1, buffer.length || 1);
  const duration: any = Math.max(.001, finite(buffer.duration, frames / Math.max(1, finite(buffer.sampleRate, 1))));
  const length: any = Math.max(1, Math.min(frames, Math.ceil(duration * rate)));
  const peaks: any = new Float32Array(length);
  const channels: any = Math.max(1, buffer.numberOfChannels || 1);
  const data: any = [];
  for (let channel: any = 0; channel < channels; channel++) data.push(buffer.getChannelData(channel));
  for (let bucket: any = 0; bucket < length; bucket++) {
    const start: any = Math.floor(bucket * frames / length);
    const end: any = bucket === length - 1 ? frames : Math.max(start + 1, Math.floor((bucket + 1) * frames / length));
    let peak: any = 0;
    for (const samples of data) {
      for (let index: any = start; index < end; index++) peak = Math.max(peak, Math.abs(samples[index] || 0));
    }
    peaks[bucket] = peak;
  }
  return peaks;
}

function decodedBytes(buffer: any) {
  return Math.max(0, finite(buffer && buffer.length) * Math.max(1, finite(buffer && buffer.numberOfChannels, 1)) * 4);
}

function activeAssetIds() {
  return new Set([
    ...state.pinnedAssets,
    ...[...state.voices.values()].map((voice: any) => voice.assetId),
  ]);
}

function trimDecodedCache() {
  if (!PM.assets || !PM.assets.map) return;
  const decoded: any = [...PM.assets.map.values()].filter((asset: any) => asset && asset.kind === 'audio' && asset.audioBuffer);
  let total: any = decoded.reduce((sum: any, asset: any) => sum + decodedBytes(asset.audioBuffer), 0);
  if (total <= MAX_DECODED_BYTES || decoded.length < 2) return;
  const active: any = activeAssetIds();
  decoded.sort((a: any, b: any) => finite(a.audioUsedAt) - finite(b.audioUsedAt));
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

async function decodeAsset(asset: any) {
  if (!asset || asset.kind !== 'audio') throw new Error('Audio asset is unavailable');
  if (asset.audioBuffer) {
    asset.audioUsedAt = Date.now();
    return asset.audioBuffer;
  }
  if (asset.audioDecoding) return asset.audioDecoding;
  if (!asset.audioBlob) throw new Error('Audio media is missing · import it again to relink it');
  if (asset.audioRetryAt > Date.now() && asset.audioDecodeError) throw asset.audioDecodeError;
  const token: any = asset.audioToken;
  asset.audioDecoding = (async () => {
    try {
      const bytes: any = await asset.audioBlob.arrayBuffer();
      const buffer: any = await decodeArrayBuffer(context(), bytes.slice(0));
      if (!buffer || !(buffer.duration > 0)) throw new Error('The decoder returned an empty audio file');
      if (decodedBytes(buffer) > MAX_DECODED_BYTES) {
        const error: any = new Error('This audio file is too long to decode safely · use a shorter edit or split it first');
        error.audioUserFacing = true;
        throw error;
      }
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
    } catch (error: any) {
      const failure: any = error && (error.audioUserFacing || /missing|decoder|supported/i.test(error.message || ''))
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

async function prepareAsset({ id, name, blob, meta = {} }: any) {
  const asset: any = {
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

function disposeAsset(asset: any) {
  if (!asset || asset.kind !== 'audio') return;
  asset.audioDisposed = true;
  asset.audioToken = Symbol('disposed-audio-asset');
  for (const [id, voice] of state.voices) if (voice.assetId === asset.id) stopVoice(id);
  asset.audioBuffer = null;
  asset.peaks = null;
  asset.audioBlob = null;
  for (const [id, request] of state.decodeRequests) if (request.asset === asset) state.decodeRequests.delete(id);
}

function gainAt(layer: any, localTime: any, audibleDuration: any) {
  const data: any = layer && layer.d || {};
  const base: any = clamp(finite(data.gain, 1), 0, 4);
  const duration: any = Math.max(0, finite(audibleDuration, finite(layer && layer.dur)));
  const local: any = clamp(finite(localTime), 0, duration);
  const fadeIn: any = Math.max(0, finite(data.fadeIn));
  const fadeOut: any = Math.max(0, finite(data.fadeOut));
  const entering: any = fadeIn > 0 ? clamp(local / fadeIn, 0, 1) : 1;
  const leaving: any = fadeOut > 0 ? clamp((duration - local) / fadeOut, 0, 1) : 1;
  return base * Math.min(entering, leaving);
}

function envelope(layer: any, localStart: any, duration: any, samples: any = 96, audibleDuration: any) {
  const count: any = Math.max(2, Math.floor(samples));
  const values: any = new Float32Array(count);
  for (let index: any = 0; index < count; index++) {
    values[index] = gainAt(layer, localStart + duration * index / (count - 1), audibleDuration);
  }
  return values;
}

/* gainAt() is piecewise linear. Scheduling only the exact slope changes keeps a
   100 ms fade at 100 ms even when the clip lasts for minutes. */
function envelopePoints(layer: any, localStart: any, duration: any, audibleDuration: any) {
  const data: any = layer && layer.d || {};
  const full: any = Math.max(0, finite(audibleDuration, finite(layer && layer.dur)));
  const start: any = clamp(finite(localStart), 0, full);
  const end: any = clamp(start + Math.max(0, finite(duration)), start, full);
  const fadeIn: any = Math.max(0, finite(data.fadeIn));
  const fadeOut: any = Math.max(0, finite(data.fadeOut));
  const times: any = [start, end];
  if (fadeIn > start + 1e-6 && fadeIn < end - 1e-6) times.push(fadeIn);
  const fadeOutStart: any = full - fadeOut;
  if (fadeOut > 0 && fadeOutStart > start + 1e-6 && fadeOutStart < end - 1e-6) times.push(fadeOutStart);
  if (fadeIn > 0 && fadeOut > 0 && fadeIn > fadeOutStart) {
    const crossing: any = full * fadeIn / (fadeIn + fadeOut);
    if (crossing > start + 1e-6 && crossing < end - 1e-6) times.push(crossing);
  }
  return [...new Set(times)].sort((a: any, b: any) => a - b).map((local: any) => ({
    local,
    value: gainAt(layer, local, full),
  }));
}

function plan(layer: any, asset: any, rangeStart: any, rangeEnd: any) {
  if (!layer || layer.type !== 'audio' || layer.on === false || !layer.d || !layer.d.asset || !asset) return null;
  const layerStart: any = Math.max(0, finite(layer.from));
  const layerDuration: any = Math.max(0, finite(layer.dur));
  const windowStart: any = Math.max(layerStart, finite(layer._audioWindowStart, layerStart));
  const windowEnd: any = Math.min(layerStart + layerDuration, finite(layer._audioWindowEnd, layerStart + layerDuration));
  const sourceDuration: any = asset.audioBuffer ? asset.audioBuffer.duration : finite(asset.dur);
  const trim: any = Math.max(0, finite(layer.d.trim));
  const audibleDuration: any = Math.min(layerDuration, Math.max(0, sourceDuration - trim));
  const clipStart: any = windowStart;
  const clipEnd: any = Math.min(windowEnd, layerStart + audibleDuration);
  const start: any = Math.max(clipStart, finite(rangeStart));
  const end: any = Math.min(clipEnd, finite(rangeEnd, clipEnd));
  if (end - start <= 1e-4) return null;
  const localStart: any = start - layerStart;
  const sourceOffset: any = trim + localStart;
  if (!(sourceDuration > 0) || sourceOffset >= sourceDuration - 1e-4) return null;
  const duration: any = Math.min(end - start, sourceDuration - sourceOffset);
  if (duration <= 1e-4) return null;
  return { layer, asset, start, end: start + duration, duration, localStart, sourceOffset, audibleDuration };
}

function voiceSignature(layer: any, asset: any) {
  const data: any = layer.d || {};
  return [asset.id, asset.audioToken && String(asset.audioToken), finite(layer.from), finite(layer.dur),
    finite(data.trim), finite(data.gain, 1), finite(data.fadeIn), finite(data.fadeOut)].join('|');
}

function publishState() {
  if (typeof window.document === 'undefined' || !window.document.documentElement) return;
  const root: any = window.document.documentElement;
  root.dataset.audioEngine = 'decoded-buffer-v2';
  root.dataset.audioState = state.running ? 'playing' : 'paused';
  root.dataset.audioVoices = String(state.voices.size);
  const first: any = state.voices.values().next().value;
  if (first) root.dataset.audioSourceOffset = String(Math.round(first.sourceOffset * 1000) / 1000);
  else delete root.dataset.audioSourceOffset;
}

function stopVoice(id: any) {
  const voice: any = state.voices.get(id);
  if (!voice) return;
  state.voices.delete(id);
  voice.ended = true;
  try { voice.source.onended = null; voice.source.stop(); } catch (error: any) { }
  try { voice.source.disconnect(); } catch (error: any) { }
  try { voice.gain.disconnect(); } catch (error: any) { }
  publishState();
}

function stopAll() {
  for (const id of [...state.voices.keys()]) stopVoice(id);
  publishState();
}

function applyEnvelope(param: any, layer: any, localStart: any, duration: any, at: any, audibleDuration: any) {
  const points: any = envelopePoints(layer, localStart, duration, audibleDuration);
  try {
    param.cancelScheduledValues(at);
    param.setValueAtTime(points[0].value, at);
    if (duration > .002 && typeof param.linearRampToValueAtTime === 'function') {
      for (let index: any = 1; index < points.length; index++) {
        param.linearRampToValueAtTime(points[index].value, at + points[index].local - localStart);
      }
    } else if (duration > .002 && typeof param.setValueCurveAtTime === 'function') {
      param.setValueCurveAtTime(envelope(layer, localStart, duration, 256, audibleDuration), at, duration);
    }
  } catch (error: any) { param.value = points[0].value; }
  return points;
}

function startVoice(clip: any, transportTime: any) {
  const ctx: any = context();
  const source: any = ctx.createBufferSource();
  const gain: any = ctx.createGain();
  source.buffer = clip.asset.audioBuffer;
  source.connect(gain).connect(state.master);
  /* A clip whose in-point is later than the current playhead must be queued on
     the audio clock, not started early. Once the playhead is inside the clip,
     plan() sets clip.start to transportTime and this delay naturally becomes 0. */
  const at: any = ctx.currentTime + Math.max(0, clip.start - finite(transportTime));
  const curve: any = applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at, clip.audibleDuration);
  const voice: any = {
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
    try { source.disconnect(); } catch (error: any) { }
    try { gain.disconnect(); } catch (error: any) { }
    publishState();
  };
  try { source.start(at, clip.sourceOffset, clip.duration); }
  catch (error: any) { stopVoice(clip.layer.id); return null; }
  clip.asset.audioUsedAt = Date.now();
  publishState();
  return voice;
}

function requestDecode(layer: any, asset: any, generation: any) {
  if (asset.audioRetryAt > Date.now()) return;
  const existing: any = state.decodeRequests.get(layer.id);
  if (existing && existing.asset === asset && existing.generation === generation) return;
  const request: any = { asset, generation };
  state.decodeRequests.set(layer.id, request);
  decodeAsset(asset).then((buffer: any) => {
    if (!buffer || !state.running || generation !== state.generation) return;
    const current: any = audioLayers().find((item: any) => item.id === layer.id);
    if (!current || current.d.asset !== asset.id || !PM.assets || PM.assets.get(asset.id) !== asset) return;
    sync(PM.time, true);
  }).catch((error: any) => {
    if (generation === state.generation && PM.toast) PM.toast(error.message || 'Could not decode audio', 5000);
  }).finally(() => {
    if (state.decodeRequests.get(layer.id) === request) state.decodeRequests.delete(layer.id);
  });
}

function voiceDrift(voice: any, layer: any, time: any, ctx: any) {
  if (!voice || !ctx) return Infinity;
  const audioUntilStart: any = voice.scheduledAt - ctx.currentTime;
  const transportUntilStart: any = voice.transportStart - time;
  if (audioUntilStart > 0 || transportUntilStart > 0) {
    return Math.abs(audioUntilStart - transportUntilStart);
  }
  const audioSourceTime: any = voice.sourceOffset + Math.max(0, ctx.currentTime - voice.scheduledAt);
  const transportSourceTime: any = Math.max(0, finite(layer.d && layer.d.trim) + time - finite(layer.from));
  return Math.abs(audioSourceTime - transportSourceTime);
}

function sync(time: any, force: any = false) {
  if (!state.running || !PM.proj) { stopAll(); return; }
  const generation: any = state.generation;
  const desired: any = new Set();
  for (const layer of audioLayers()) {
    const audibleStart: any = Math.max(finite(layer.from), finite(layer._audioWindowStart, finite(layer.from)));
    if (audibleStart - time > LIVE_LOOKAHEAD) continue;
    const asset: any = layer.d && layer.d.asset && PM.assets && PM.assets.get(layer.d.asset);
    const clip: any = plan(layer, asset, time, layer.from + layer.dur);
    if (!clip) continue;
    desired.add(layer.id);
    if (!asset.audioBuffer) {
      stopVoice(layer.id);
      requestDecode(layer, asset, generation);
      continue;
    }
    const voice: any = state.voices.get(layer.id);
    const signature: any = voiceSignature(layer, asset);
    const drifted: any = voice && voiceDrift(voice, layer, time, state.context) > DRIFT_TOLERANCE;
    if (force || !voice || voice.signature !== signature || drifted) {
      stopVoice(layer.id);
      startVoice(clip, time);
    }
  }
  for (const id of [...state.voices.keys()]) if (!desired.has(id)) stopVoice(id);
  publishState();
}

function start(time: any) {
  state.running = true;
  state.project = PM.proj || null;
  const generation: any = ++state.generation;
  let ctx: any;
  try { ctx = context(); }
  catch (error: any) { state.running = false; publishState(); PM.toast && PM.toast(error.message, 5000); return; }
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

function seek(time: any) {
  if (!state.running) return;
  state.generation++;
  sync(time, true);
}

function tick(time: any) { sync(time, false); }

function reconcile() {
  state.project = PM.proj || null;
  if (!PM.playing) { pause(); return; }
  state.generation++;
  sync(PM.time, true);
}

function reconcileProject() {
  const changed: any = state.projectGeneration !== PM.projGeneration;
  state.projectGeneration = PM.projGeneration;
  if (!changed) return;
  reconcile();
}

function drawWaveform(ctx: any, layer: any, options: any = {}) {
  const x: any = finite(options.x);
  const y: any = finite(options.y);
  const width: any = Math.max(0, finite(options.width, finite(options.w)));
  const height: any = Math.max(0, finite(options.height, finite(options.h)));
  const clipLeft: any = finite(options.clipLeft, x);
  const asset: any = layer && layer.d && PM.assets && PM.assets.get(layer.d.asset);
  const left: any = Math.max(x, clipLeft);
  const right: any = x + width;
  if (!(right > left && height > 2)) return false;
  if (!asset || !asset.peaks || !asset.peaks.length) {
    if (asset && asset.kind === 'audio' && !asset.audioDecoding && !(asset.audioRetryAt > Date.now())) {
      decodeAsset(asset).catch(() => {});
    }
    ctx.fillStyle = options.placeholderColor || 'rgba(255,255,255,.22)';
    ctx.fillRect(left, y + height / 2, right - left, 1);
    return false;
  }
  const peaks: any = asset.peaks;
  const duration: any = Math.max(.001, finite(asset.dur));
  const trim: any = Math.max(0, finite(layer.d.trim));
  const step: any = Math.max(1, finite(options.step, 1));
  /* anchor 0.5 mirrors around the middle; 1 stands the bars on the floor. */
  const anchor: any = clamp(finite(options.anchor, .5), 0, 1);
  const minBar: any = Math.max(0, finite(options.minBar, 1));
  ctx.fillStyle = options.color || 'rgba(255,255,255,.52)';
  const secondsPerPx: any = Math.max(0, finite(layer.dur)) / Math.max(1, width);
  const perSecond: any = peaks.length / duration;
  for (let px: any = left; px < right; px += step) {
    const sourceTime: any = trim + (px - x) * secondsPerPx;
    if (sourceTime >= duration) continue;
    /* One column summarizes every peak under it, so zooming out keeps the
       envelope's shape instead of sampling a random bucket. */
    const from: any = Math.min(peaks.length - 1, Math.floor(clamp(sourceTime / duration, 0, 1) * peaks.length));
    const to: any = Math.min(peaks.length, Math.max(from + 1, Math.floor((sourceTime + step * secondsPerPx) * perSecond)));
    let peak: any = 0;
    for (let index: any = from; index < to; index++) peak = Math.max(peak, peaks[index] || 0);
    const bar: any = Math.max(minBar, peak * height);
    ctx.fillRect(px, y + (height - bar) * anchor, step, bar);
  }
  return true;
}

async function collectDecodedClips(t0: any, t1: any) {
  const from: any = Math.max(0, finite(t0));
  const to: any = Math.max(from, finite(t1, from));
  if (to - from <= 1e-4 || !PM.proj) return { from, to, clips: [], pinned: new Set() };
  const clips: any = [];
  const pinned: any = new Set();
  try {
    for (const layer of audioLayers()) {
      const layerStart: any = Math.max(finite(layer.from), finite(layer._audioWindowStart, finite(layer.from)));
      const layerEnd: any = Math.min(finite(layer.from) + Math.max(0, finite(layer.dur)),
        finite(layer._audioWindowEnd, finite(layer.from) + Math.max(0, finite(layer.dur))));
      if (Math.min(to, layerEnd) - Math.max(from, layerStart) <= 1e-4) continue;
      const asset: any = layer.d && layer.d.asset && PM.assets && PM.assets.get(layer.d.asset);
      if (!asset) throw new Error(`Audio media is missing for “${layer.name || 'Audio'}” · import it again before exporting`);
      /* Known metadata lets us skip out-of-range or fully trimmed clips without
         decoding an unrelated file just because it exists in the project. */
      if (asset.dur > 0 && !plan(layer, asset, from, to)) continue;
      state.pinnedAssets.add(asset.id);
      pinned.add(asset.id);
      try { await decodeAsset(asset); }
      catch (error: any) {
        throw new Error(`Could not include “${layer.name || asset.name || 'Audio'}” in the export · ${error.message || 'decode failed'}`);
      }
      const clip: any = plan(layer, asset, from, to);
      if (clip) clips.push(clip);
    }
    return { from, to, clips, pinned };
  } catch (error: any) {
    releasePins(pinned);
    throw error;
  }
}

function releasePins(pinned: any) {
  for (const id of pinned || []) state.pinnedAssets.delete(id);
  trimDecodedCache();
}

async function renderOffline(t0: any, t1: any) {
  const prepared: any = await collectDecodedClips(t0, t1);
  try {
    const { from, to, clips }: any = prepared;
    if (!clips.length) return null;
    const Offline: any = window.OfflineAudioContext;
    if (!Offline) throw new Error('Offline audio rendering is not supported on this system');
    const sampleRate: any = 48000;
    let offline: any;
    try { offline = new Offline(2, Math.ceil((to - from) * sampleRate), sampleRate); }
    catch (error: any) { throw new Error('Could not create the audio export mix'); }
    for (const clip of clips) {
      try {
        const source: any = offline.createBufferSource();
        const gain: any = offline.createGain();
        source.buffer = clip.asset.audioBuffer;
        source.connect(gain).connect(offline.destination);
        const at: any = clip.start - from;
        applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at, clip.audibleDuration);
        source.start(at, clip.sourceOffset, clip.duration);
      } catch (error: any) { throw new Error(`Could not mix “${clip.layer.name || clip.asset.name || 'Audio'}” for export`); }
    }
    try { return await offline.startRendering(); }
    catch (error: any) { throw new Error('Audio mixdown failed · try realtime capture'); }
  } finally {
    releasePins(prepared.pinned);
  }
}

/* MediaRecorder needs a real audio MediaStream. This path schedules the same
   decoded clips and exact envelope points as preview/offline export, but routes
   them to an isolated stream destination instead of the speakers. */
async function createRealtimeMix(t0: any, t1: any) {
  const prepared: any = await collectDecodedClips(t0, t1);
  try {
    const { from, clips }: any = prepared;
    if (!clips.length) return null;
    const ctx: any = context();
    if (typeof ctx.createMediaStreamDestination !== 'function') {
      throw new Error('Realtime audio export is not supported on this system');
    }
    if (ctx.state === 'suspended' && typeof ctx.resume === 'function') await ctx.resume();
    const destination: any = ctx.createMediaStreamDestination();
    const nodes: any = [];
    for (const clip of clips) {
      const source: any = ctx.createBufferSource();
      const gain: any = ctx.createGain();
      source.buffer = clip.asset.audioBuffer;
      source.connect(gain).connect(destination);
      nodes.push({ source, gain, clip });
    }
    let started: any = false;
    let stopped: any = false;
    return {
      stream: destination.stream,
      start(leadSeconds: any = .06) {
        if (started || stopped) return 0;
        started = true;
        const lead: any = Math.max(.02, finite(leadSeconds, .06));
        const base: any = ctx.currentTime + lead;
        for (const { source, gain, clip } of nodes) {
          const at: any = base + clip.start - from;
          applyEnvelope(gain.gain, clip.layer, clip.localStart, clip.duration, at, clip.audibleDuration);
          source.start(at, clip.sourceOffset, clip.duration);
        }
        return lead;
      },
      stop() {
        if (stopped) return;
        stopped = true;
        for (const { source, gain } of nodes) {
          try { source.stop(); } catch (error: any) { }
          try { source.disconnect(); } catch (error: any) { }
          try { gain.disconnect(); } catch (error: any) { }
        }
        for (const track of destination.stream.getTracks ? destination.stream.getTracks() : []) {
          try { track.stop(); } catch (error: any) { }
        }
      },
    };
  } finally {
    releasePins(prepared.pinned);
  }
}

function opusHead(channels: any, sampleRate: any) {
  const out: any = new Uint8Array(19);
  out.set([79, 112, 117, 115, 72, 101, 97, 100], 0); // OpusHead
  out[8] = 1;
  out[9] = channels;
  out[10] = 56; out[11] = 1; // 312-sample pre-skip, little endian
  new DataView(out.buffer).setUint32(12, sampleRate, true);
  out[18] = 0;
  return out;
}

async function supportsOpus() {
  const Encoder: any = window.AudioEncoder;
  const Data: any = window.AudioData;
  if (!Encoder || !Data || typeof Encoder.isConfigSupported !== 'function') return false;
  const config: any = { codec: 'opus', sampleRate: 48000, numberOfChannels: 2, bitrate: 160000 };
  const support: any = await Encoder.isConfigSupported(config).catch(() => null);
  return !!(support && support.supported);
}

async function encodeOpus(buffer: any) {
  const Encoder: any = window.AudioEncoder;
  const Data: any = window.AudioData;
  if (!buffer || !Encoder || !Data) return null;
  const channels: any = Math.min(2, Math.max(1, buffer.numberOfChannels || 1));
  const config: any = { codec: 'opus', sampleRate: buffer.sampleRate, numberOfChannels: channels, bitrate: 160000 };
  const support: any = await Encoder.isConfigSupported(config).catch(() => null);
  if (!support || !support.supported) return null;
  const chunks: any = [];
  let privateData: any = null;
  let encodeError: any = null;
  const encoder: any = new Encoder({
    output(chunk: any, metadata: any) {
      const description: any = metadata && metadata.decoderConfig && metadata.decoderConfig.description;
      if (!privateData && description) privateData = new Uint8Array(description);
      const bytes: any = new Uint8Array(chunk.byteLength);
      chunk.copyTo(bytes);
      chunks.push({ ts: Number(chunk.timestamp), data: bytes });
    },
    error(error: any) { encodeError = error; },
  });
  encoder.configure(config);
  const source: any = Array.from({ length: channels }, (_: any, channel: any) => buffer.getChannelData(channel));
  const frameLength: any = Math.max(1, Math.floor(buffer.sampleRate * .02));
  for (let offset: any = 0; offset < buffer.length; offset += frameLength) {
    const frames: any = Math.min(frameLength, buffer.length - offset);
    const planar: any = new Float32Array(frames * channels);
    for (let channel: any = 0; channel < channels; channel++) planar.set(source[channel].subarray(offset, offset + frames), channel * frames);
    const data: any = new Data({
      format: 'f32-planar',
      sampleRate: buffer.sampleRate,
      numberOfFrames: frames,
      numberOfChannels: channels,
      timestamp: Math.round(offset / buffer.sampleRate * 1e6),
      data: planar,
    });
    encoder.encode(data);
    data.close();
    if (encoder.encodeQueueSize > 16) await new Promise((resolve: any) => setTimeout(resolve, 4));
  }
  await encoder.flush();
  encoder.close();
  if (encodeError || !chunks.length) return null;
  return { chunks, priv: privateData || opusHead(channels, buffer.sampleRate), rate: buffer.sampleRate, channels };
}

function hasAudibleLayers(project: any = PM.proj) {
  return audioLayers(project).some((layer: any) => layer.on !== false && layer.d && layer.d.asset);
}

const Audio: any = PM.Audio = {
  accepts,
  normalizeLayer,
  prepareAsset,
  disposeAsset,
  decodeAsset,
  rebalanceCache: trimDecodedCache,
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
    voices: [...state.voices.values()].map((voice: any) => ({
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
}
