#!/usr/bin/env node
// The SDR color bar against AVFoundation at full precision (Phase 3b gate,
// docs/engine/phase3b-gate.md). Renders `pm-export`'s bar chart (the same 40
// patches, 1280 × 640, 8 × 5, solids at their top-left corners, dither off)
// through `pm-cli render` (the job engine's export pipeline) to H.264, HEVC
// 10-bit, and ProRes 422 HQ, then measures each movie with
// sdr-avfoundation.swift: AVAssetReader's half-float and 8-bit BGRA decodes of
// frame 0, patch-center ΔE2000 against the analytic sRGB codes. The bar:
// maximum 1.0 and mean 0.5 at 8-bit, maximum 0.5 at 10-bit. Prints one JSON
// line per movie. No Electron, no window.
//
//   node apps/desktop/scripts/gate-3b/sdr-avfoundation.mjs [--keep]
//
// Needs `engine/target/release/pm-cli` (cargo build --release -p pm-cli).

import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../../..');
const cli = path.join(repo, 'engine/target/release/pm-cli');
const KEEP = process.argv.includes('--keep');

// pm-export tests/bars_delivery.rs CHART: ColorChecker-like patches, a grey
// ramp, and primaries and secondaries.
const CHART = [[115,82,68],[194,150,130],[98,122,157],[87,108,67],[133,128,177],[103,189,170],[214,126,44],[80,91,166],[193,90,99],[94,60,108],[157,188,64],[224,163,46],[56,61,150],[70,148,73],[175,54,60],[231,199,31],[187,86,149],[8,133,161],[243,243,242],[200,200,200],[160,160,160],[122,122,121],[85,85,85],[52,52,52],[0,0,0],[28,28,28],[56,56,56],[84,84,84],[113,113,113],[141,141,141],[170,170,170],[198,198,198],[226,226,226],[255,255,255],[255,0,0],[0,255,0],[0,0,255],[0,255,255],[255,0,255],[255,255,0]];
const W = 1280, H = 640, COLS = 8, ROWS = 5;

const ch = (v) => ({ v, kf: [], expr: null });
function layer(id, d, x, y) {
  const p = {};
  for (const [k, v] of [['anchor.x', 0], ['anchor.y', 0], ['position.x', x], ['position.y', y], ['scale.x', 100], ['scale.y', 100], ['rotation', 0], ['opacity', 100], ['skew', 0]]) p[k] = ch(v);
  return { id, type: 'solid', name: id, from: 0, dur: 1, on: true, solo: false, blend: 'normal', mblur: false, parent: null, group: null, matteSource: null, threeD: false, p, fx: [], masks: [], d, transitionIn: null, transitionOut: null };
}
const hex = (c) => '#' + c.map((v) => v.toString(16).padStart(2, '0')).join('');
const pw = W / COLS, ph = H / ROWS;
const project = {
  id: 'P1', name: 'chart', w: W, h: H, fps: 24000 / 1001, dur: 1, work: [0, 1], compId: 'P1-root',
  backgroundFill: { type: 'none', angle: 0, stops: [{ id: 's', color: '#000000', position: 0 }] },
  layers: CHART.map((c, i) => layer(`p${i}`, { color: hex(c), w: pw, h: ph }, (i % COLS) * pw, Math.floor(i / COLS) * ph)),
  assets: {}, comps: {}
};

const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-gate-3b-sdr-'));
const input = path.join(dir, 'chart.json');
const codes = path.join(dir, 'codes.json');
writeFileSync(input, JSON.stringify(project));
writeFileSync(codes, JSON.stringify(CHART));
let pass = true;
for (const [format, depth, ext] of [['h264', 8, 'mp4'], ['hevc10', 10, 'mp4'], ['prores-hq', 10, 'mov']]) {
  const out = path.join(dir, `${format}.${ext}`);
  // Two frames, as the bar test exports.
  const r = spawnSync(cli, ['render', input, '-o', out, '--format', format, '--fps', '24000/1001', '--range', '0:0.0834', '--dither', 'off', '--no-audio'], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`pm-cli render ${format}: ${r.stderr}${r.stdout}`);
  const m = spawnSync('swift', [path.join(here, 'sdr-avfoundation.swift'), out, String(W), String(H), String(COLS), String(ROWS), codes], { encoding: 'utf8' });
  if (m.status !== 0) throw new Error(`swift ${format}: ${m.stderr}`);
  const measured = JSON.parse(m.stdout);
  const bar = depth === 8 ? (v) => v.max <= 1.0 && v.mean <= 0.5 : (v) => v.max <= 0.5;
  const ok = bar(measured.half);
  pass &&= ok;
  console.log(JSON.stringify({ format, depth, avfoundationHalf: measured.half, avfoundationBgra8: measured.bgra8, bar: depth === 8 ? 'max ≤ 1.0, mean ≤ 0.5' : 'max ≤ 0.5', pass: ok }));
}
if (KEEP) console.error(`kept ${dir}`);
else rmSync(dir, { recursive: true, force: true });
process.exit(pass ? 0 : 1);
