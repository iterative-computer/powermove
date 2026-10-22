/**
 * Audio parity references from the legacy engine: the export mix
 * (`PM.Audio.renderOffline`, the path `exporter.ts` takes for every export
 * with audio) of the corpus's audio project and of audio projects built here,
 * captured from the built legacy app.
 *
 *   bun engine/crates/pm-audio/tools/legacy-mix.ts [--root <dir>] [--id <id>]
 *
 * Run it through the machine-wide Electron lock
 * (`pm-electron-lock bun engine/crates/pm-audio/tools/legacy-mix.ts`).
 * Writes `<root>/audio-<stamp>-<commit>/` (default root: the corpus root,
 * ~/Library/Application Support/Powermove Engine Corpus), never inside the
 * repository:
 *
 *   manifest.json                    projects, media, the app's decode rate
 *   media/                           generated media (clicks, tones, layouts)
 *   projects/<name>/document.json    as built (media: references)
 *   projects/<name>/loaded.json      PM.proj after import (asset ids)
 *   projects/<name>/legacy.wav       renderOffline(0, dur): 32-bit float
 *   real/media/                      audio extracted from the real project (private,
 *                                    stays under the corpus root)
 *
 * `--real <file.pmv>` (or the `PM_AUDIO_REAL_PROJECT` environment variable)
 * adds a real project kept outside the repository; `--skip-real` leaves it
 * out. Without either, no real project is captured.
 *
 * The Rust side (`tests/legacy_parity.rs`) renders the same documents with
 * pm-audio and compares (docs/engine/pm-audio.md).
 */
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { launchLegacyApp, type LegacyApp } from '../../../corpus/src/app';
import { repoRoot } from '../../../corpus/src/catalog';
import { newProject, type Builder, type Doc } from '../../../corpus/src/doc';
import { resolveMedia, ref } from '../../../corpus/src/media';
import { installCorpusPage } from '../../../corpus/src/page';
import { allProjects } from '../../../corpus/src/projects';

const DEFAULT_ROOT = path.join(os.homedir(), 'Library/Application Support/Powermove Engine Corpus');

interface AudioProject { name: string; doc: Doc; notes: string; clicks?: Array<{ media: string; frames: number[]; fps: [number, number]; from: number; trim: number; rate: number }> }

// ── media ────────────────────────────────────────────────────────────────

/** A WAV (WAVE_FORMAT_EXTENSIBLE, 32-bit float) from planar channels. */
function wav(rate: number, channels: Float32Array[], mask: number): Uint8Array {
  const frames = channels[0]!.length, n = channels.length;
  const data = frames * n * 4;
  const buf = new ArrayBuffer(68 + data);
  const v = new DataView(buf);
  const str = (at: number, s: string) => { for (let i = 0; i < s.length; i++) v.setUint8(at + i, s.charCodeAt(i)); };
  str(0, 'RIFF'); v.setUint32(4, 60 + data, true); str(8, 'WAVE');
  str(12, 'fmt '); v.setUint32(16, 40, true); v.setUint16(20, 0xFFFE, true); v.setUint16(22, n, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate * n * 4, true); v.setUint16(32, n * 4, true); v.setUint16(34, 32, true);
  v.setUint16(36, 22, true); v.setUint16(38, 32, true); v.setUint32(40, mask, true);
  // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
  const guid = [0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];
  guid.forEach((b, i) => v.setUint8(44 + i, b));
  str(60, 'data'); v.setUint32(64, data, true);
  for (let i = 0; i < frames; i++) for (let c = 0; c < n; c++) v.setFloat32(68 + (i * n + c) * 4, channels[c]![i]!, true);
  return new Uint8Array(buf);
}

const STEREO = 0x3, MONO = 0x4, SIDE51 = 0x60F;

/** A plain WAVE_FORMAT_IEEE_FLOAT WAV (no channel mask), as most tools write mono. */
function wavPlain(rate: number, channels: Float32Array[]): Uint8Array {
  const frames = channels[0]!.length, n = channels.length;
  const data = frames * n * 4;
  const buf = new ArrayBuffer(44 + data);
  const v = new DataView(buf);
  const str = (at: number, s: string) => { for (let i = 0; i < s.length; i++) v.setUint8(at + i, s.charCodeAt(i)); };
  str(0, 'RIFF'); v.setUint32(4, 36 + data, true); str(8, 'WAVE');
  str(12, 'fmt '); v.setUint32(16, 16, true); v.setUint16(20, 3, true); v.setUint16(22, n, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate * n * 4, true); v.setUint16(32, n * 4, true); v.setUint16(34, 32, true);
  str(36, 'data'); v.setUint32(40, data, true);
  for (let i = 0; i < frames; i++) for (let c = 0; c < n; c++) v.setFloat32(44 + (i * n + c) * 4, channels[c]![i]!, true);
  return new Uint8Array(buf);
}

/** Our FFmpeg build's CLI, for compressed mono media (absent: those are skipped). */
const FFMPEG = path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffmpeg');

/** Mono sources the mono project plays, in order, 0.5 s each. */
const MONO_SOURCES = ['mono48-plain.wav', 'mono48-fc.wav', 'mono48-nomask.wav', 'mono22-fc.wav', 'mono-aac.m4a', 'mono-opus.ogg', 'mono-flac.flac'];

function clicks(rate: number, seconds: number, at: number[], amplitude = 0.9): Float32Array {
  const x = new Float32Array(Math.round(rate * seconds));
  for (const i of at) x[i] = amplitude;
  return x;
}

function tone(rate: number, seconds: number, hz: number, amplitude: number): Float32Array {
  const x = new Float32Array(Math.round(rate * seconds));
  for (let i = 0; i < x.length; i++) x[i] = amplitude * Math.sin(2 * Math.PI * hz * i / rate);
  return x;
}

/** Deterministic white noise (a 32-bit LCG): every band, so a resampler or
 * an interpolation that differs anywhere shows. */
function noise(rate: number, seconds: number, amplitude: number, seed: number): Float32Array {
  const x = new Float32Array(Math.round(rate * seconds));
  let state = seed >>> 0;
  for (let i = 0; i < x.length; i++) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    x[i] = amplitude * (state / 2147483648 - 1);
  }
  return x;
}

/** Frame N at fps num/den lands at sample floor(N × rate × den / num). */
const frameSample = (n: number, rate: number, [num, den]: [number, number]) => Math.floor(n * rate * den / num);

function generateMedia(dir: string): Record<string, string> {
  mkdirSync(dir, { recursive: true });
  const files: Record<string, Uint8Array> = {};
  const f30: [number, number] = [30, 1], f2997: [number, number] = [30000, 1001];
  const c30 = [1, 7, 20, 45].map(n => frameSample(n, 48000, f30));
  files['click48.wav'] = wav(48000, [clicks(48000, 2, c30), clicks(48000, 2, c30, -0.9)], STEREO);
  const c30b = [3, 11].map(n => frameSample(n, 48000, f30));
  files['click48b.wav'] = wav(48000, [clicks(48000, 1.5, c30b, 0.7), clicks(48000, 1.5, c30b, 0.7)], STEREO);
  const c2997 = [1, 2, 30, 59].map(n => frameSample(n, 48000, f2997));
  files['click48-2997.wav'] = wav(48000, [clicks(48000, 2.2, c2997), clicks(48000, 2.2, c2997)], STEREO);
  const c44 = [2, 10, 33].map(n => frameSample(n, 44100, f30));
  files['click44.wav'] = wav(44100, [clicks(44100, 1.5, c44), clicks(44100, 1.5, c44)], STEREO);
  files['tone440.wav'] = wav(48000, [tone(48000, 4, 440, 0.5), tone(48000, 4, 440, 0.5)], STEREO);
  files['tone550.wav'] = wav(48000, [tone(48000, 4, 550, 0.4), tone(48000, 4, 550, 0.4)], STEREO);
  files['tone660.wav'] = wav(48000, [tone(48000, 4, 660, 0.4), tone(48000, 4, 660, 0.3)], STEREO);
  files['tone44k.wav'] = wav(44100, [tone(44100, 3, 1000, 0.5), tone(44100, 3, 1500, 0.5)], STEREO);
  // Full-band noise at the rates people import, for the decode resampler.
  for (const rate of [22050, 32000, 44100, 88200, 96000]) {
    files[`noise${rate}.wav`] = wav(rate, [noise(rate, 1.2, 0.4, rate), noise(rate, 1.2, 0.4, rate + 1)], STEREO);
  }
  files['noise48.wav'] = wav(48000, [noise(48000, 3, 0.4, 7), noise(48000, 3, 0.4, 8)], STEREO);
  files['mono22k.wav'] = wav(22050, [tone(22050, 3, 330, 0.6)], MONO);
  files['surround.wav'] = wav(48000, [300, 400, 500, 60, 700, 800].map(hz => tone(48000, 3, hz, 0.2)), SIDE51);
  // Mono in the ways files carry it: plain, with a center mask, with no mask, at 22.05 kHz.
  files['mono48-plain.wav'] = wavPlain(48000, [tone(48000, 1, 440, 0.5)]);
  files['mono48-fc.wav'] = wav(48000, [tone(48000, 1, 440, 0.5)], MONO);
  files['mono48-nomask.wav'] = wav(48000, [tone(48000, 1, 440, 0.5)], 0);
  files['mono22-fc.wav'] = wav(22050, [tone(22050, 1, 440, 0.5)], MONO);
  const out: Record<string, string> = {};
  for (const [name, bytes] of Object.entries(files)) {
    const file = path.join(dir, name);
    writeFileSync(file, bytes);
    out[name] = file;
  }
  if (existsSync(FFMPEG)) {
    const plain = out['mono48-plain.wav']!;
    for (const [name, codec] of [['mono-aac.m4a', ['-c:a', 'aac', '-b:a', '128k']], ['mono-opus.ogg', ['-c:a', 'libopus', '-b:a', '96k']], ['mono-flac.flac', ['-c:a', 'flac']]] as const) {
      const file = path.join(dir, name);
      execFileSync(FFMPEG, ['-hide_banner', '-loglevel', 'error', '-i', plain, ...codec, '-y', file]);
      out[name] = file;
    }
  }
  return out;
}

// ── projects ─────────────────────────────────────────────────────────────

function audioLayer(b: Builder, media: string, d: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) {
  return b.layer('audio', { name: media, d: { asset: ref(media), gain: 1, trim: 0, fadeIn: 0, fadeOut: 0, ...d }, ...extra });
}

function projects(): AudioProject[] {
  const out: AudioProject[] = [];
  const corpus = allProjects().find(p => p.id === 'media/audio');
  if (!corpus) throw new Error('the corpus has no media/audio project');
  out.push({ name: 'corpus-media-audio', doc: corpus.doc, notes: 'The corpus audio project: five codecs with gains, trims, fades, and keyframed embedded audio.' });

  let b = newProject('audio/clicks-30', { name: 'Clicks at 30 fps', fps: 30, dur: 2 });
  b.add(audioLayer(b, 'click48.wav'));
  b.add(audioLayer(b, 'click48b.wav', { trim: 0.1, gain: 0.5 }, { from: 0.5, dur: 1.4 }));
  out.push({
    name: 'clicks-30', doc: b.root, notes: 'Single-sample clicks on frames; a second clip offset and trimmed.',
    clicks: [
      { media: 'click48.wav', frames: [1, 7, 20, 45], fps: [30, 1], from: 0, trim: 0, rate: 48000 },
      { media: 'click48b.wav', frames: [3, 11], fps: [30, 1], from: 0.5, trim: 0.1, rate: 48000 },
    ],
  });

  b = newProject('audio/clicks-2997', { name: 'Clicks at 29.97 fps', fps: 29.97, dur: 2.2 });
  b.add(audioLayer(b, 'click48-2997.wav'));
  out.push({ name: 'clicks-2997', doc: b.root, notes: 'Clicks on 29.97 frames.', clicks: [{ media: 'click48-2997.wav', frames: [1, 2, 30, 59], fps: [30000, 1001], from: 0, trim: 0, rate: 48000 }] });

  b = newProject('audio/clicks-44k', { name: 'Clicks from a 44.1 kHz file', fps: 30, dur: 1.5 });
  b.add(audioLayer(b, 'click44.wav'));
  out.push({ name: 'clicks-44k', doc: b.root, notes: 'Clicks resampled from 44.1 kHz.', clicks: [{ media: 'click44.wav', frames: [2, 10, 33], fps: [30, 1], from: 0, trim: 0, rate: 44100 }] });

  b = newProject('audio/gain-fades', { name: 'Gain keyframes, fades, master', fps: 30, dur: 3 });
  b.root.audioGain = 0.8;
  b.add(audioLayer(b, 'tone440.wav', { gain: b.ch(0.2, [[0, 0.2], [1, 1.5], [2, 0.5]]), fadeIn: 0.3, fadeOut: 0.5 }));
  b.add(audioLayer(b, 'tone550.wav', { gain: b.ch(0.7, [[0.5, 0.1, { legacy: 'easeInOut' }], [2.5, 1, { legacy: 'easeInOut' }]]), trim: 0.25 }, { from: 0.2, dur: 2.6 }));
  out.push({ name: 'gain-fades', doc: b.root, notes: 'Linear and eased gain keyframes, fades, master gain 0.8.' });

  b = newProject('audio/precomp-nesting', { name: 'Precomp nesting', fps: 30, dur: 3 });
  const inner = b.composition({ name: 'Inner', fps: 30, dur: 3 });
  inner.audioGain = 0.7;
  b.within(inner, () => b.add(audioLayer(b, 'tone660.wav', { trim: 0.1, gain: 0.9 }, { from: 0.25, dur: 2.5 }), inner));
  b.add(b.layer('precomp', { name: 'Inner A', d: { comp: inner.id, audioGain: 0.5 } }));
  b.add(b.layer('precomp', { name: 'Inner B', from: 0.5, dur: 2.5, d: { comp: inner.id, trim: 0.2, audioGain: b.ch(1, [[0, 1], [1.5, 0.25]]) } }));
  out.push({ name: 'precomp-nesting', doc: b.root, notes: 'One composition used twice: offsets, trims, precomp and composition levels.' });

  b = newProject('audio/solo', { name: 'Solo', fps: 30, dur: 2 });
  b.add(audioLayer(b, 'tone440.wav', {}, { solo: true }));
  b.add(audioLayer(b, 'tone550.wav'));
  out.push({ name: 'solo', doc: b.root, notes: 'A soloed layer silences the other.' });

  b = newProject('audio/mute-groups', { name: 'Mute and groups', fps: 30, dur: 2 });
  const group = b.add(b.layer('group', { name: 'Group' }));
  group.on = b.ch(true, [[0, true, { hold: true }], [1, false, { hold: true }]]);
  b.add(audioLayer(b, 'tone660.wav', {}, { group: group.id }));
  b.add(audioLayer(b, 'tone550.wav', {}, { on: false }));
  b.add(audioLayer(b, 'tone440.wav', { gain: 0.5 }, { on: b.ch(true, [[0, true, { hold: true }], [0.5, false, { hold: true }], [1.5, true, { hold: true }]]) }));
  out.push({ name: 'mute-groups', doc: b.root, notes: 'A muted layer, a group switched off at 1 s, a layer switched off from 0.5 to 1.5 s.' });

  b = newProject('audio/layouts', { name: 'Layouts and rates', fps: 30, dur: 2.5 });
  b.add(audioLayer(b, 'mono22k.wav', { gain: 0.8 }));
  b.add(audioLayer(b, 'surround.wav', { gain: 0.9 }));
  b.add(audioLayer(b, 'tone44k.wav', { gain: 0.5 }));
  out.push({ name: 'layouts', doc: b.root, notes: 'Mono 22.05 kHz, 5.1 at 48 kHz, stereo 44.1 kHz.' });

  b = newProject('audio/mono', { name: 'Mono sources', fps: 30, dur: MONO_SOURCES.length * 0.5 });
  MONO_SOURCES.forEach((name, i) => b.add(audioLayer(b, name, {}, { from: i * 0.5, dur: 0.5 })));
  out.push({ name: 'mono', doc: b.root, notes: `Mono sources in turn, 0.5 s each: ${MONO_SOURCES.join(', ')}.` });

  // The app's own shape (PM.P channels on every field, as the export
  // encoding parity spec builds its automation project): gain keyframes,
  // fades and trims as channels, a nested composition with channel levels.
  b = newProject('audio/app-channels', { name: 'App channels', fps: 30, dur: 1 });
  const nestedComp = b.composition({ name: 'Nested audio', fps: 30, dur: 1 });
  nestedComp.audioGain = b.ch(0.7);
  b.within(nestedComp, () => b.add(audioLayer(b, 'tone440.wav', { gain: b.ch(0.35), trim: b.ch(0.1), speed: b.ch(1), fadeIn: b.ch(0.08), fadeOut: b.ch(0.1) }, { from: 0, dur: 1 }), nestedComp));
  b.add(audioLayer(b, 'tone550.wav', { gain: b.ch(0.25, [[0, 0.25], [0.4, 0.9], [0.85, 0.4]]), trim: b.ch(0.05), fadeIn: b.ch(0.14), fadeOut: b.ch(0.2) }, { from: 0.1, dur: 0.85 }));
  b.add(b.layer('precomp', { name: 'Nested mix', from: 0, dur: 1, d: { comp: nestedComp.id, audioGain: b.ch(0.75), trim: b.ch(0) } }));
  out.push({ name: 'app-channels', doc: b.root, notes: 'Every content field a channel, a nested composition, gain keyframes.' });

  // Today's envelope steps one frame apart (at least 30 a second) along
  // eased keys and animated fades, from where each clip starts.
  for (const fps of [24, 60]) {
    b = newProject(`audio/eased-${fps}`, { name: `Eased levels at ${fps} fps`, fps, dur: 2.5 });
    b.add(audioLayer(b, 'noise48.wav', { gain: b.ch(0.2, [[0.1, 0.2, { legacy: 'easeInOut' }], [1.2, 1.4, { legacy: 'expoOut' }], [2, 0.3, { legacy: 'backOut' }]]), fadeIn: b.ch(0.2, [[0, 0.2], [1, 0.4]]), fadeOut: b.ch(0.35) }, { from: 0.05, dur: 2.3 }));
    b.add(audioLayer(b, 'tone660.wav', { gain: b.ch(0.6), trim: b.ch(0.2), fadeIn: b.ch(0.5), fadeOut: b.ch(0.5) }, { from: 0.4, dur: 1.2 }));
    out.push({ name: `eased-${fps}`, doc: b.root, notes: `Eased gain keys and an animated fade in at ${fps} fps.` });
  }

  // In-points and starts between samples: today's source node interpolates
  // a start between frames and rounds the offset to a whole sample.
  b = newProject('audio/fractional', { name: 'Fractional starts', fps: 30, dur: 2 });
  b.add(audioLayer(b, 'noise48.wav', { trim: 0.0104167, gain: 0.6 }, { from: 0.123457, dur: 0.7654321 }));
  b.add(audioLayer(b, 'tone440.wav', { trim: 0.333333, gain: 0.5, fadeIn: 0.0123 }, { from: 1.0000104, dur: 0.9 }));
  const shifted = b.composition({ name: 'Shifted', fps: 30, dur: 2 });
  b.within(shifted, () => b.add(audioLayer(b, 'tone550.wav', { trim: 0.0201, gain: 0.7 }, { from: 0.0313, dur: 1.2 }), shifted));
  b.add(b.layer('precomp', { name: 'Shifted', from: 0.40007, dur: 1.5, d: { comp: shifted.id, trim: 0.0507, audioGain: 0.9 } }));
  out.push({ name: 'fractional', doc: b.root, notes: 'Clip starts, trims and precomposition offsets between samples.' });

  // Every common import rate through the decode resampler.
  b = newProject('audio/rates', { name: 'Sample rates', fps: 30, dur: 3.2 });
  [22050, 32000, 44100, 88200, 96000].forEach((rate, i) => b.add(audioLayer(b, `noise${rate}.wav`, { trim: 0.05, gain: 0.8 }, { from: i * 0.6, dur: 0.9 })));
  out.push({ name: 'rates', doc: b.root, notes: 'Full-band noise at 22.05, 32, 44.1, 88.2 and 96 kHz, overlapping.' });

  b = newProject('audio/embedded', { name: 'Embedded audio', fps: 30, dur: 2 });
  b.add(b.layer('video', { name: 'MOV soundtrack', from: 0.2, dur: 1.6, d: { asset: ref('h264-aac.mov'), fit: 'contain', w: 320, h: 180, trim: 0.3, embeddedAudio: true, audioGain: 0.8 } }));
  out.push({ name: 'embedded', doc: b.root, notes: 'A video soundtrack, offset and trimmed.' });
  return out;
}

// ── capture ──────────────────────────────────────────────────────────────

function mediaRefs(value: unknown, out = new Set<string>()): Set<string> {
  if (typeof value === 'string') { if (value.startsWith('media:')) out.add(value.slice(6)); }
  else if (Array.isArray(value)) value.forEach(item => mediaRefs(item, out));
  else if (value && typeof value === 'object') Object.values(value).forEach(item => mediaRefs(item, out));
  return out;
}

/** `PM.Audio.renderOffline(0, dur)` in the app, as planar float channels. */
async function renderLegacy(legacy: LegacyApp, dur: number): Promise<{ sampleRate: number; length: number; planar: Float32Array[]; decodeRates: Record<string, number> }> {
  const mix: any = await legacy.evaluate(async (d: number) => {
    const PM = (window as any).PM;
    const buffer = await PM.Audio.renderOffline(0, d);
    if (!buffer) return null;
    const channels: string[] = [];
    for (let c = 0; c < buffer.numberOfChannels; c++) {
      const bytes = new Uint8Array(buffer.getChannelData(c).buffer.slice(0));
      let text = '';
      for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      channels.push(btoa(text));
    }
    const rates: Record<string, number> = {};
    for (const [assetId, asset] of PM.assets.map) if (asset?.sampleRate) rates[assetId] = asset.sampleRate;
    return { sampleRate: buffer.sampleRate, length: buffer.length, channels, decodeRates: rates };
  }, dur);
  if (!mix) throw new Error('renderOffline returned no mix');
  const planar = mix.channels.map((b64: string) => new Float32Array(new Uint8Array(Buffer.from(b64, 'base64')).buffer));
  return { sampleRate: mix.sampleRate, length: mix.length, planar, decodeRates: mix.decodeRates };
}

/** Opens a saved project in the app, extracts the audio its layers use, and captures its mix. */
async function captureReal(legacy: LegacyApp, out: string, file: string): Promise<Record<string, any>> {
  const slug = 'real-' + path.basename(file, path.extname(file)).toLowerCase().replace(/[^a-z0-9]+/g, '-');
  const dir = path.join(out, 'projects', slug);
  mkdirSync(dir, { recursive: true });
  const record: Record<string, any> = { name: slug, notes: 'A real project (outside the repository).', clicks: [], media: {} };
  try {
    const copy = path.join(legacy.userData, path.basename(file));
    copyFileSync(file, copy);
    await legacy.evaluateMain(({ dialog }: any, filePath: string) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, copy);
    const before = await legacy.evaluate(() => (window as any).PM.proj.id);
    await legacy.evaluate(() => (window as any).PM.openProject());
    await legacy.waitFor((id: string) => (window as any).PM.proj.id !== id, before, 120_000);
    await legacy.evaluate(async () => { await (window as any).PM.projectAssetsReady; });
    record.mediaPending = await legacy.evaluate(() => (window as any).__corpus.waitForMedia(180000));
    const loaded = await legacy.evaluate(() => JSON.stringify((window as any).PM.proj));
    writeFileSync(path.join(dir, 'loaded.json'), loaded + '\n');
    const doc = JSON.parse(loaded);
    record.dur = doc.dur;
    record.fps = doc.fps;
    const media: Array<{ id: string; name: string; data: string }> = await legacy.evaluate(async () => {
      const PM = (window as any).PM;
      const ids = new Set<string>();
      const visit = (comp: any) => { for (const l of comp?.layers || []) if (l?.d?.asset && (l.type === 'audio' || (l.type === 'video' && l.d.embeddedAudio))) ids.add(l.d.asset); };
      visit(PM.proj);
      for (const c of Object.values<any>(PM.proj.comps || {})) visit(c);
      const outList: Array<{ id: string; name: string; data: string }> = [];
      for (const id of ids) {
        const asset = PM.assets.get(id);
        const blob = asset?.audioBlob;
        if (!blob) continue;
        const bytes = new Uint8Array(await blob.arrayBuffer());
        let text = '';
        for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
        outList.push({ id, name: String(asset.name || id), data: btoa(text) });
      }
      return outList;
    });
    const mediaDir = path.join(out, 'real', 'media');
    mkdirSync(mediaDir, { recursive: true });
    for (const m of media) {
      const ext = path.extname(m.name) || '.bin';
      const target = path.join(mediaDir, `${m.id}${ext}`);
      writeFileSync(target, Buffer.from(m.data, 'base64'));
      record.media[m.name] = { file: target, assetId: m.id, kind: 'audio' };
    }
    const mix = await renderLegacy(legacy, doc.dur);
    writeFileSync(path.join(dir, 'legacy.wav'), wav(mix.sampleRate, mix.planar, mix.planar.length === 1 ? MONO : STEREO));
    record.legacy = { sampleRate: mix.sampleRate, length: mix.length, channels: mix.planar.length, decodeRates: mix.decodeRates };
  } catch (error) {
    record.error = String((error as Error).message || error).split('\n')[0];
  }
  return record;
}

async function main() {
  const args = process.argv.slice(2);
  const opt = (name: string) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : undefined; };
  const root = path.resolve(opt('--root') ?? DEFAULT_ROOT);
  const commit = execFileSync('git', ['rev-parse', '--short=8', 'HEAD'], { cwd: repoRoot, encoding: 'utf8' }).trim();
  const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
  const id = opt('--id') ?? `audio-${stamp}-${commit}`;
  const out = path.join(root, id);
  if (path.resolve(out).startsWith(repoRoot + path.sep)) throw new Error('refusing to write inside the repository');
  if (existsSync(out)) throw new Error(`${out} exists`);
  mkdirSync(out, { recursive: true });
  const generated = generateMedia(path.join(out, 'media'));
  const fileOf = (name: string) => generated[name] ?? (() => { const m = resolveMedia(name, out); return path.isAbsolute(m.path) ? m.path : path.join(repoRoot, m.path); })();

  const manifest: Record<string, any> = { format: 'powermove-audio-parity', version: 1, id, commit, started: new Date().toISOString(), projects: [] };
  const legacy = await launchLegacyApp();
  try {
    await legacy.evaluate(installCorpusPage);
    for (const project of projects()) {
      const dir = path.join(out, 'projects', project.name);
      mkdirSync(dir, { recursive: true });
      writeFileSync(path.join(dir, 'document.json'), JSON.stringify(project.doc, null, 1) + '\n');
      const record: Record<string, any> = { name: project.name, notes: project.notes, dur: project.doc.dur, fps: project.doc.fps, clicks: project.clicks ?? [], media: {} };
      try {
        await legacy.evaluate(doc => (window as any).__corpus.open(doc), project.doc);
        const names = [...mediaRefs(project.doc)];
        await legacy.evaluate(() => (window as any).__corpus.mediaInput());
        await legacy.setInputFiles('#corpus-media', names.map(fileOf));
        const imported: Record<string, { id: string; kind: string; error?: string }> = await legacy.evaluate(() => (window as any).__corpus.importMedia());
        const map: Record<string, string> = {};
        for (const name of names) {
          const r = imported[name];
          record.media[name] = { file: fileOf(name), assetId: r?.id ?? '', kind: r?.kind ?? '', ...(r?.error ? { error: r.error } : {}) };
          if (r?.id) map[name] = r.id;
        }
        const { project: rewritten } = await legacy.evaluate(m => (window as any).__corpus.rewriteMedia(m), map);
        await legacy.evaluate(doc => (window as any).__corpus.open(doc), rewritten);
        record.mediaPending = await legacy.evaluate(() => (window as any).__corpus.waitForMedia(30000));
        const loaded = await legacy.evaluate(() => JSON.stringify((window as any).PM.proj));
        writeFileSync(path.join(dir, 'loaded.json'), loaded + '\n');
        const mix = await renderLegacy(legacy, project.doc.dur);
        const planar = mix.planar;
        writeFileSync(path.join(dir, 'legacy.wav'), wav(mix.sampleRate, planar, planar.length === 1 ? MONO : STEREO));
        record.legacy = { sampleRate: mix.sampleRate, length: mix.length, channels: planar.length, decodeRates: mix.decodeRates };
      } catch (error) {
        record.error = String((error as Error).message || error).split('\n')[0];
      }
      manifest.projects.push(record);
      console.log(`${project.name}: ${record.error ?? `${record.legacy?.length} samples`}`);
    }
    const real = opt('--real') ?? process.env.PM_AUDIO_REAL_PROJECT;
    if (!args.includes('--skip-real') && real && existsSync(real)) {
      const record = await captureReal(legacy, out, real);
      manifest.projects.push(record);
      console.log(`${record.name}: ${record.error ?? `${record.legacy?.length} samples`}`);
    }
  } finally {
    manifest.pageErrors = legacy.pageErrors;
    await legacy.close();
  }
  manifest.finished = new Date().toISOString();
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify(manifest, null, 1) + '\n');
  console.log(out);
}

await main();
