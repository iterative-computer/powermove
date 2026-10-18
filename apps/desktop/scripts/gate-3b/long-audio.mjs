#!/usr/bin/env node
// The audio bar's "No length limit" (plan section 2) through the export
// path: a long composition (default 75 minutes at 29.97 fps, 16 × 16 px) with
// a streamed tone under its whole length and single-sample clicks at frames
// near the start, the middle, and the end, rendered by `pm-cli render` (the
// job engine's pipeline: pm-audio's offline mix, AVAssetWriter, verification)
// to ProRes Proxy with 32-bit float PCM. Checks, on FFmpeg's decode of the
// file: total samples equal round(duration × sr), and every click lands at
// floor(N × sr × den / num) ± 1. Prints one JSON line. No Electron.
//
//   node apps/desktop/scripts/gate-3b/long-audio.mjs [--minutes 75] [--keep]
//
// Needs `engine/target/release/pm-cli` (cargo build --release -p pm-cli) and
// our FFmpeg build (`PM_FFMPEG_DIR`, as engine/env.sh sets it).

import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../../..');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const MINUTES = Number(opt('--minutes', '75'));
const KEEP = args.includes('--keep');
const NUM = 30000, DEN = 1001, SR = 48000;
const ffdir = process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out');
const ffmpeg = path.join(ffdir, 'bin/ffmpeg');
const cli = path.join(repo, 'engine/target/release/pm-cli');

const frames = Math.round((MINUTES * 60 * NUM) / DEN);
const duration = (frames * DEN) / NUM;
const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-gate-3b-long-audio-'));

// A mono 32-bit float WAV at 48 kHz with one full-scale click at sample 0.
function clickWav(file, seconds) {
  const n = Math.round(seconds * SR);
  const b = Buffer.alloc(44 + n * 4);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 4, 4); b.write('WAVEfmt ', 8);
  b.writeUInt32LE(16, 16); b.writeUInt16LE(3, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(SR, 24); b.writeUInt32LE(SR * 4, 28); b.writeUInt16LE(4, 32); b.writeUInt16LE(32, 34);
  b.write('data', 36); b.writeUInt32LE(n * 4, 40);
  b.writeFloatLE(1, 44);
  writeFileSync(file, b);
}

const ch = (v) => ({ v, kf: [], expr: null });
function layer(id, type, d, from, dur) {
  const p = {};
  for (const [k, v] of [['anchor.x', 0], ['anchor.y', 0], ['position.x', 0], ['position.y', 0], ['scale.x', 100], ['scale.y', 100], ['rotation', 0], ['opacity', 100], ['skew', 0]]) p[k] = ch(v);
  return { id, type, name: id, from, dur, on: true, solo: false, blend: 'normal', mblur: false, parent: null, group: null, matteSource: null, threeD: false, p, fx: [], masks: [], d, transitionIn: null, transitionOut: null };
}

const t0 = Date.now();
const click = path.join(dir, 'click.wav');
clickWav(click, 0.5);
// The bed: a quiet 220 Hz tone as long as the composition, FLAC (streamed by
// pm-audio; nothing holds the whole file).
const bed = path.join(dir, 'bed.flac');
const gen = spawnSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', `aevalsrc=0.01*sin(2*PI*220*t):s=${SR}:d=${duration + 1}`, '-ac', '1', '-c:a', 'flac', bed]);
if (gen.status !== 0) throw new Error(`ffmpeg bed: ${gen.stderr}`);

const clickFrames = [37, Math.floor(frames / 2) + 11, frames - 61];
const layers = clickFrames.map((n, i) => layer(`click${i}`, 'audio', { asset: 'click', gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 }, (n * DEN) / NUM, 0.5));
layers.push(layer('bed', 'audio', { asset: 'bed', gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 }, 0, duration));
layers.push(layer('bg', 'solid', { color: '#000000', w: 16, h: 16 }, 0, duration));
const project = {
  id: 'P1', name: 'long audio', w: 16, h: 16, fps: NUM / DEN, dur: duration, work: [0, duration], compId: 'P1-root',
  backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 's', color: '#000000', position: 0 }] },
  layers,
  assets: {
    click: { id: 'click', name: 'click.wav', kind: 'audio', format: 'wav', dur: 0.5, sourcePath: click },
    bed: { id: 'bed', name: 'bed.flac', kind: 'audio', format: 'flac', dur: duration + 1, sourcePath: bed }
  },
  comps: {}
};
const input = path.join(dir, 'long.json');
writeFileSync(input, JSON.stringify(project));
const output = path.join(dir, 'long.mov');
const r = spawnSync(cli, ['render', input, '-o', output, '--format', 'prores-proxy', '--fps', `${NUM}/${DEN}`, '--audio', 'float'], { encoding: 'utf8', maxBuffer: 64 << 20 });
if (r.status !== 0) throw new Error(`pm-cli render exited ${r.status}: ${r.stderr}\n${r.stdout}`);
const rendered = JSON.parse(r.stdout.slice(r.stdout.indexOf('{')));
const renderSeconds = (Date.now() - t0) / 1000;

// Stream FFmpeg's decode (first channel, f32) and find the clicks.
const want = clickFrames.map((n) => Math.floor((n * SR * DEN) / NUM));
const found = [];
let total = 0, carry = Buffer.alloc(0);
await new Promise((resolve, reject) => {
  const dec = spawn(ffmpeg, ['-v', 'error', '-i', output, '-map', '0:a:0', '-af', 'pan=mono|c0=c0', '-f', 'f32le', '-']);
  dec.stdout.on('data', (chunk) => {
    const buf = carry.length ? Buffer.concat([carry, chunk]) : chunk;
    const whole = buf.length - (buf.length % 4);
    for (let o = 0; o < whole; o += 4) {
      if (buf.readFloatLE(o) > 0.5) found.push(total);
      total++;
    }
    carry = buf.subarray(whole);
  });
  dec.on('error', reject);
  dec.on('close', (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg decode exited ${code}`))));
});

const expectedTotal = Math.round(duration * SR);
const clicks = want.map((w, i) => ({ frame: clickFrames[i], expected: w, at: found[i] ?? null }));
const result = {
  minutes: MINUTES,
  frames,
  durationSeconds: duration,
  bytes: rendered.bytes,
  renderFps: rendered.fps,
  renderSeconds,
  audioSamplesReported: rendered.audioSamples,
  audioSamplesDecoded: total,
  audioSamplesExpected: expectedTotal,
  clicks,
  clicksFound: found.length,
  pass: total === expectedTotal && rendered.audioSamples === expectedTotal && found.length === want.length && clicks.every((c) => c.at !== null && Math.abs(c.at - c.expected) <= 1)
};
console.log(JSON.stringify(result));
if (KEEP) console.error(`kept ${dir}`);
else rmSync(dir, { recursive: true, force: true });
process.exit(result.pass ? 0 : 1);
