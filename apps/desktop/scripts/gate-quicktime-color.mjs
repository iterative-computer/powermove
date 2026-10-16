/**
 * Section 2 color invariant (3), Standard color, for the Phase 2 gate
 * (docs/engine/phase2-gate.md). A 40-patch chart (the 24 ColorChecker Classic
 * patches in X-Rite's sRGB values, a grey ramp with shadow steps, and the
 * primaries and secondaries; the chart of spike S2) is built as solid layers,
 * exported through `pm-cli render` (the job engine's pipeline: pm-render,
 * VideoToolbox, AVAssetWriter, verification) with the "Rec.709 (QuickTime and
 * Safari)" output (`--color quicktime`, CICP 1-13-1, limited range) as
 * H.264, ProRes 422 HQ, and ProRes 4444, then each movie is played through
 * AVPlayerItemVideoOutput and compared with the viewer's ColorSync conversion
 * of the same codes by gate-quicktime-color.swift. Bar: ΔE2000 at most 1.0 on
 * every patch, judged on the float (RGhA) delivery; 8-bit BGRA is reported.
 *
 *   source engine/env.sh && (cd engine && cargo build --release -p pm-cli)
 *   node apps/desktop/scripts/gate-quicktime-color.mjs [--json out.json]
 *
 * Everything is written to a temporary folder that is removed afterwards.
 * Opens no window; runs no Electron.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const repo = path.resolve(import.meta.dirname, '../../..');
const pmCli = path.join(repo, 'engine/target/release/pm-cli');
if (!existsSync(pmCli)) throw new Error(`build pm-cli first: ${pmCli}`);

const CHART = [
  [115, 82, 68], [194, 150, 130], [98, 122, 157], [87, 108, 67], [133, 128, 177], [103, 189, 170],
  [214, 126, 44], [80, 91, 166], [193, 90, 99], [94, 60, 108], [157, 188, 64], [224, 163, 46],
  [56, 61, 150], [70, 148, 73], [175, 54, 60], [231, 199, 31], [187, 86, 149], [8, 133, 161],
  [243, 243, 242], [200, 200, 200], [160, 160, 160], [122, 122, 121], [85, 85, 85], [52, 52, 52],
  [0, 0, 0], [3, 3, 3], [8, 8, 8], [16, 16, 16], [24, 24, 24], [32, 32, 32], [64, 64, 64],
  [128, 128, 128], [192, 192, 192], [255, 255, 255],
  [255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0],
];
const [W, H, COLS, ROWS] = [1920, 1080, 8, 5];
const [PW, PH] = [W / COLS, H / ROWS];

const hex = (c) => '#' + c.map((v) => v.toString(16).padStart(2, '0')).join('').toUpperCase();
const ch = (v) => ({ v, kf: [], expr: null });
function solid(i) {
  const p = {};
  const x = (i % COLS) * PW, y = Math.floor(i / COLS) * PH;
  for (const [k, v] of Object.entries({ 'anchor.x': 0, 'anchor.y': 0, 'position.x': x, 'position.y': y, 'scale.x': 100, 'scale.y': 100, rotation: 0, opacity: 100, skew: 0 })) p[k] = ch(v);
  return {
    id: `patch-${i}`, type: 'solid', name: `patch ${i}`, from: 0, dur: 1, on: true, solo: false,
    blend: 'normal', mblur: false, parent: null, group: null, matteSource: null, threeD: false,
    p, fx: [], masks: [], d: { color: hex(CHART[i]), w: PW, h: PH }, transitionIn: null, transitionOut: null,
  };
}
const project = {
  id: 'P1', name: 'chart', w: W, h: H, fps: 24, dur: 1, work: [0, 1], compId: 'P1-root',
  backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 's', color: '#000000', position: 0 }] },
  layers: CHART.map((_, i) => solid(i)), assets: {}, comps: {},
};

const work = mkdtempSync(path.join(os.tmpdir(), 'pm-gate-qt-'));
const out = { bar: 'ΔE2000 ≤ 1.0 on every patch, AVPlayerItemVideoOutput into the display profile against ColorSync of the viewer', exports: [] };
try {
  writeFileSync(path.join(work, 'chart.json'), JSON.stringify(project));
  writeFileSync(path.join(work, 'codes.json'), JSON.stringify(CHART));
  for (const [format, file] of [['h264', 'chart.mp4'], ['prores-hq', 'chart-hq.mov'], ['prores-4444', 'chart-4444.mov']]) {
    const movie = path.join(work, file);
    execFileSync(pmCli, ['render', path.join(work, 'chart.json'), '-o', movie, '--format', format, '--color', 'quicktime', '--no-audio'], { stdio: ['ignore', 'pipe', 'inherit'] });
    const probe = execFileSync(path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffprobe'),
      ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=codec_name,pix_fmt,color_range,color_primaries,color_transfer,color_space', '-of', 'json', movie]).toString();
    const report = JSON.parse(execFileSync('swift', [path.join(import.meta.dirname, 'gate-quicktime-color.swift'), movie, String(W), String(H), String(COLS), String(ROWS), path.join(work, 'codes.json')], { maxBuffer: 64 << 20, stdio: ['ignore', 'pipe', 'inherit'] }).toString());
    const stream = JSON.parse(probe).streams[0];
    for (const c of report.cases) delete c.patches;
    out.exports.push({ format, stream, displayProfile: report.displayProfile, cases: report.cases });
    for (const c of report.cases) {
      process.stdout.write(`${format} (${stream.codec_name} ${stream.pix_fmt}, ${stream.color_primaries}/${stream.color_transfer}/${stream.color_space}/${stream.color_range}) ${c.output} → ${c.display}: max ΔE2000 ${c.maxDeltaE.toFixed(3)}, mean ${c.meanDeltaE.toFixed(3)}, worst ${JSON.stringify(c.worstPatch)} ${c.passMax1 ? 'pass' : 'FAIL'}\n`);
    }
  }
  // Judged on the float delivery (as spike S2 defined it): the bar is about the
  // display conversion, and an 8-bit BGRA delivery of an 8-bit H.264 rounds
  // once more on the way (reported, not judged).
  out.judgedOn = 'RGhA (64RGBAHalf)';
  out.pass = out.exports.every((e) => e.cases.filter((c) => c.output === out.judgedOn).every((c) => c.passMax1));
  process.stdout.write(`invariant (3), judged on ${out.judgedOn}: ${out.pass ? 'pass' : 'FAIL'}\n`);
  const i = process.argv.indexOf('--json');
  if (i > 0) writeFileSync(process.argv[i + 1], JSON.stringify(out, null, 2) + '\n');
} finally {
  if (process.argv.includes('--keep')) process.stdout.write(`kept ${work}\n`); else rmSync(work, { recursive: true, force: true });
}
