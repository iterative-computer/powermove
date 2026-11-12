// V2 viewer frames for the visual parity sheet (README.md): what a person
// sees paused in the running lane app at each sample time.
//
//   node tools/visual-parity/viewer-frames.mjs --pid <lane app pid> --window <window id>
//        --rect x,y,w,h --out <dir> [--times 2,8,13.458]
//
// Seeks through the timeline's Current time field with the cua-driver CLI
// (background input, no focus change), waits until the viewer stops changing, and crops
// the viewer canvas (`--rect`, in the window screenshot's pixels) from a
// native-resolution window screenshot into <out>/v2viewer/<time>.png. Times
// come from <out>/v1/manifest.json (or v2/ without v1), all of them or the
// samples nearest `--times` (seconds). The viewer shows the frame at its zoom,
// so this column is for looking, not for pixel statistics.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { timeName } from './lib.mjs';

const argv = process.argv.slice(2);
const opt = {};
for (let i = 0; i < argv.length; i += 2) opt[argv[i].replace(/^--/, '')] = argv[i + 1];
const cua = path.join(process.env.HOME, '.local/bin/cua-driver');
const pid = Number(opt.pid), window = Number(opt.window);
const [x, y, w, h] = opt.rect.split(',').map(Number);
const out = path.resolve(opt.out, 'v2viewer');
mkdirSync(out, { recursive: true });
const base = JSON.parse(readFileSync(['v1', 'v2'].map((d) => path.resolve(opt.out, d, 'manifest.json')).find(existsSync), 'utf8'));
const fps = base.project.fps;
const wanted = opt.times
  ? opt.times.split(',').map(Number).map((t) => base.samples.reduce((a, b) => (Math.abs(b.time - t) < Math.abs(a.time - t) ? b : a)))
  : base.samples;
const session = `parity-${process.pid}`;
const call = (tool, args) => JSON.parse(execFileSync(cua, ['call', tool, JSON.stringify({ pid, window_id: window, session, ...args })], { encoding: 'utf8', maxBuffer: 64 << 20 }));
const steps = (list) => {
  const r = call('run_steps', { steps: list });
  if (r.failed_step !== null) throw new Error(`run_steps failed: ${JSON.stringify(r).slice(0, 400)}`);
};
const field = (role) => {
  const t = call('get_window_state', { include_screenshot: false, query: 'Current time' });
  const m = new RegExp(`\\[(\\d+)\\] ${role} "Current time"(?: = "([^"]*)")?`).exec(t.tree_markdown);
  return m ? { token: `${t.snapshot_id}:${m[1]}`, value: m[2] ?? '' } : null;
};
// The time button reads "13 seconds, 11 frames" (total seconds).
const spoken = (t) => {
  const f = Math.round(t * fps), s = Math.floor(f / fps), ff = f - s * fps;
  return `${s} second${s === 1 ? "" : "s"}, ${ff} frame${ff === 1 ? "" : "s"}`;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const timecode = (t) => {
  const f = Math.round(t * fps), s = Math.floor(f / fps), ff = f - s * fps;
  return [Math.floor(s / 3600), Math.floor(s / 60) % 60, s % 60, ff].map((n) => String(n).padStart(2, '0')).join(':');
};

execFileSync(cua, ['call', 'start_session', JSON.stringify({ session })]);
const samples = [];
const shot = path.join(out, '.window.png');
let before = null;
for (const s of wanted) {
  const button = field('AXButton');
  if (button) steps([{ tool: 'click', args: { element_token: button.token } }]);
  const text = field('AXTextField');
  if (!text) throw new Error('no Current time field');
  steps([{ tool: 'set_value', args: { element_token: text.token, value: timecode(s.time) } }]);
  steps([{ tool: 'press_key', args: { key: 'return' } }]);
  // The playhead is there once the time button reads the target.
  for (let tries = 0; ; tries++) {
    const now = field('AXButton');
    if (now && (now.value === spoken(s.time) || now.value.endsWith(`, ${spoken(s.time)}`))) break;
    if (tries > 40) throw new Error(`the playhead did not reach ${timecode(s.time)} (reads ${now?.value})`);
    await sleep(250);
  }
  // Settled: Auto has replaced its reduced first picture with the exact one
  // (the quality menu reads "currently Full") and two window screenshots
  // 1.5 s apart match (video decode and extension rasters take seconds), at
  // most 90 s.
  // The last sample's picture can stay up, already exact and still, while
  // the new frame decodes: a picture equal to it counts only after 60 s.
  const file = `${timeName(s.time)}.png`;
  let previous = null;
  const started = Date.now();
  for (const deadline = Date.now() + 90_000; ;) {
    await sleep(1500);
    const quality = call('get_window_state', { include_screenshot: false, query: 'Preview resolution' }).tree_markdown;
    if (/currently (?!Full)/.test(quality) && Date.now() < deadline) continue;
    call('get_window_state', { include_accessibility_tree: false, max_image_dimension: 0, screenshot_out_file: shot });
    execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', '-i', shot, '-vf', `crop=${w}:${h}:${x}:${y}`, path.join(out, file)]);
    const bytes = readFileSync(path.join(out, file));
    const stale = before?.equals(bytes) && Date.now() - started < 60_000;
    if ((previous && previous.equals(bytes) && !stale) || Date.now() > deadline) break;
    previous = bytes;
  }
  before = readFileSync(path.join(out, file));
  samples.push({ time: s.time, timecode: timecode(s.time), file });
  console.log(`[viewer] ${timecode(s.time)}`);
}
rmSync(shot, { force: true });
writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: 'V2 viewer, paused, window screenshot', samples }, null, 1));
