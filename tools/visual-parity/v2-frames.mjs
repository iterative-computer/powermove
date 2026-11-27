// V2 frames for the visual parity sheet (README.md), through the running
// lane app's own MCP server: export_video is the app's export job on the open
// project, with the profile's extensions and raster overrides.
//
//   node tools/visual-parity/v2-frames.mjs --app <Powermove.app> --profile <V2 profile>
//        --out <dir> --project <file.pmv> [--via export|stills]
//
// The app must already show the project (pm-gui launch). Sample times and the
// project size come from <out>/v1/manifest.json when it exists, so both
// columns hold the same instants.
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
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
  await tool('start_session', { prompt: 'Visual parity frames', context: 'project', access: 'project' });
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
  if (args.via === 'stills') {
    // An agent's stills: the export pipeline at most 1280 wide (the tool's
    // limit), with missing effects bypassed as the viewer does. v1 must be
    // captured at the same --width.
    const width = Math.min(info.w, 1280);
    const samples = [];
    for (let i = 0; i < times.length; i += 5) {
      const batch = times.slice(i, i + 5);
      const r = await tool('render_frames', { times: batch, width });
      const images = r.content.filter((c) => c.type === 'image');
      if (images.length !== batch.length) throw new Error(`render_frames returned ${images.length} images for ${batch.length} times`);
      batch.forEach((time, k) => {
        const file = `${timeName(time)}.png`;
        writeFileSync(path.join(out, file), Buffer.from(images[k].data, 'base64'));
        samples.push({ time, file });
      });
      log(`stills ${batch.map((t) => t.toFixed(3)).join(', ')}`);
    }
    writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: `V2 render_frames (${width} wide, missing effects bypassed)`, project: info, samples }, null, 1));
  } else {
    // The real export: full size ProRes through the app's export job, then the
    // sample frames by index (render_frames is capped at 1280 wide).
    let r = await tool('export_video', { name: 'visual-parity', format: 'prores', range: 'all', audio: false, motionBlur: true });
    const started = Date.now();
    for (;;) {
      const value = JSON.parse(r.content.find((c) => c.type === 'text').text);
      const file = JSON.stringify(value).match(/"([^"]+\.mov)"/)?.[1];
      if (value.status !== 'running' && file) { var movie = file; break; }
      if (value.status !== 'running') throw new Error(`export_video: ${JSON.stringify(value)}`);
      log(`exporting ${value.frames ?? '?'}/${value.totalFrames ?? '?'}`);
      r = await tool('export_video', { jobId: value.jobId });
    }
    log(`exported ${movie} in ${Math.round((Date.now() - started) / 1000)} s`);
    const frames = times.map((t) => Math.min(Math.round(t * info.fps), Math.round(info.dur * info.fps) - 1));
    const tmp = path.join(out, 'extract');
    rmSync(tmp, { recursive: true, force: true });
    mkdirSync(tmp);
    execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', movie,
      '-vf', `select='${frames.map((n) => `eq(n,${n})`).join('+')}',scale=in_color_matrix=bt709:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int+full_chroma_inp,format=rgba`,
      '-fps_mode', 'passthrough', path.join(tmp, '%04d.png')]);
    const samples = [];
    const unique = [...new Set(frames)];
    times.forEach((time, k) => {
      const file = `${timeName(time)}.png`;
      copyFileSync(path.join(tmp, `${String(unique.indexOf(frames[k]) + 1).padStart(4, '0')}.png`), path.join(out, file));
      samples.push({ time, frame: frames[k], file });
    });
    rmSync(tmp, { recursive: true, force: true });
    writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: 'V2 export_video (ProRes, full size, motion blur on), frames by index', movie, project: info, samples }, null, 1));
}
  await tool('finish_session', { commit: false }).catch(() => {});
} finally {
  child.stdin.end();
  setTimeout(() => child.kill(), 2000).unref();
}
