// V2 frames for the visual parity sheet (README.md), through the running
// lane app's own MCP server: render_frames is the app's export job on the
// open project, with the profile's extensions and raster overrides.
//
//   node tools/visual-parity/v2-frames.mjs --app <Powermove.app> --profile <V2 profile>
//        --out <dir> --project <file.pmv> [--times 13:11] [--step 2]
//
// The app must already show the project (pm-gui launch). Sample times and the
// project size come from <out>/v1/manifest.json when it exists, so both
// columns hold the same instants.
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import { parseArgs, sampleTimes, timeName } from './lib.mjs';

const args = parseArgs(process.argv.slice(2));
const out = path.resolve(args.out, 'v2');
mkdirSync(out, { recursive: true });
const bin = path.join(path.resolve(args.app), 'Contents/MacOS/powermove-native');
const child = spawn(bin, ['--powermove-mcp', '--profile', path.resolve(args.profile)], { stdio: ['pipe', 'pipe', 'inherit'] });
const waiting = new Map();
let next = 1;
readline.createInterface({ input: child.stdout }).on('line', (line) => {
  let msg;
  try { msg = JSON.parse(line); } catch { return; }
  const w = waiting.get(msg.id);
  if (!w) return;
  waiting.delete(msg.id);
  msg.error ? w.reject(new Error(JSON.stringify(msg.error))) : w.resolve(msg.result);
});
const rpc = (method, params) => new Promise((resolve, reject) => {
  const id = next++;
  waiting.set(id, { resolve, reject });
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
});
const tool = async (name, input) => {
  const r = await rpc('tools/call', { name, arguments: input });
  if (r.isError) throw new Error(`${name}: ${r.content?.map((c) => c.text).join(' ')}`);
  return r;
};
const log = (line) => console.log(`[v2 ${new Date().toISOString().slice(11, 19)}] ${line}`);

try {
  await rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'visual-parity', version: '1' } });
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  await tool('start_session', { context: 'project', access: 'project' });
  const v1Manifest = path.resolve(args.out, 'v1/manifest.json');
  let info, times;
  if (existsSync(v1Manifest)) {
    const m = JSON.parse(readFileSync(v1Manifest, 'utf8'));
    info = m.project;
    times = m.samples.map((s) => s.time);
  } else {
    const state = JSON.parse((await tool('get_project_state', { indexOnly: true })).content[0].text);
    const c = state.composition ?? state;
    info = { name: c.name, w: c.width ?? c.w, h: c.height ?? c.h, fps: c.fps, dur: c.duration ?? c.dur };
    times = sampleTimes(info, args);
  }
  log(`${info.name} ${info.w}x${info.h}, ${times.length} samples`);
  const samples = [];
  for (let i = 0; i < times.length; i += 5) {
    const batch = times.slice(i, i + 5);
    const started = Date.now();
    const r = await tool('render_frames', { times: batch, width: info.w });
    const images = r.content.filter((c) => c.type === 'image');
    if (images.length !== batch.length) throw new Error(`render_frames returned ${images.length} images for ${batch.length} times`);
    batch.forEach((time, k) => {
      const file = `${timeName(time)}.png`;
      writeFileSync(path.join(out, file), Buffer.from(images[k].data, 'base64'));
      samples.push({ time, file });
    });
    log(`${batch.map((t) => t.toFixed(3)).join(', ')} in ${Date.now() - started} ms`);
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: 'V2 render_frames (export job)', project: info, samples }, null, 1));
  await tool('finish_session', { commit: false }).catch(() => {});
} finally {
  child.stdin.end();
  setTimeout(() => child.kill(), 2000).unref();
}
