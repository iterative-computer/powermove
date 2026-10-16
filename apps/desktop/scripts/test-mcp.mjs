/** Real MCP → hidden editor → media import → frames → MP4/project smoke test.
 * Build the CLI first. All projects/media/exports stay in an isolated temp profile.
 *
 * `--desktop`: the desktop app instead (build it first with `bun run build`
 * and the engine addon with `bun run build:engine`), started hidden in a
 * background test profile with Use New Engine on, and reached the way
 * external clients reach it, through `--powermove-mcp` (plan section 22,
 * Phase 2 gate). Agent exports and render_frames go through the job engine;
 * the exports are verified with our FFmpeg. Run it under the Electron lock:
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock node scripts/test-mcp.mjs --desktop */
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile, rm, stat, readdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const desktop = process.argv.includes('--desktop');

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
// Powermove's FFmpeg build, found the way the CLI host finds it (src/server/cli.ts hostFfmpeg).
const ffmpegDir = [process.env.PM_FFMPEG_DIR, path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out')]
  .find(dir => dir && existsSync(path.join(dir, 'bin/ffmpeg')));
const ffmpeg = process.env.POWERMOVE_FFMPEG || (ffmpegDir ? path.join(ffmpegDir, 'bin/ffmpeg') : 'ffmpeg');
const ffprobe = ffmpegDir ? path.join(ffmpegDir, 'bin/ffprobe') : 'ffprobe';
const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-video-'));
const profile = path.join(root, 'profile'), exportsDir = path.join(root, 'exports');
const audio = path.join(root, 'tone.wav');
const frames = 48000, bytes = Buffer.alloc(44 + frames * 2);
bytes.write('RIFF', 0); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22);
bytes.writeUInt32LE(48000, 24); bytes.writeUInt32LE(96000, 28); bytes.writeUInt16LE(2, 32); bytes.writeUInt16LE(16, 34);
bytes.write('data', 36); bytes.writeUInt32LE(frames * 2, 40);
for (let i = 0; i < frames; i++) bytes.writeInt16LE(Math.round(5000 * Math.sin(i * Math.PI * 2 * 440 / 48000)), 44 + i * 2);
await writeFile(audio, bytes);
const desktopDir = path.join(repository, 'apps/desktop');
let app = null, appLog = '';
let child;
if (desktop) {
  const electron = createRequire(path.join(desktopDir, 'package.json'))('electron');
  assert(existsSync(path.join(desktopDir, 'out/main/index.js')), 'Build the desktop app first (bun run build).');
  assert(existsSync(path.join(desktopDir, 'native/pm_node.node')), 'Build the engine addon first (bun run build:engine).');
  const env = { ...process.env, POWERMOVE_USER_DATA: profile };
  delete env.ELECTRON_RUN_AS_NODE;
  // Hidden, never focused (background-testing.ts); the new engine on; agent exports in the temp folder.
  app = spawn(electron, ['.'], {
    cwd: desktopDir, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...env, POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '1', POWERMOVE_AGENT_EXPORTS: exportsDir, POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' }
  });
  for (const stream of [app.stdout, app.stderr]) stream.on('data', chunk => { appLog = (appLog + chunk.toString()).slice(-40000); });
  const connection = path.join(profile, 'mcp/connection.json');
  const ready = Date.now() + 120000;
  while (!existsSync(connection) || !/the job engine is ready/.test(appLog)) {
    if (app.exitCode !== null) throw new Error(`The desktop app exited (${app.exitCode}): ${appLog}`);
    if (Date.now() > ready) throw new Error(`The desktop app did not start MCP and the job engine: ${appLog}`);
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  child = spawn(electron, ['.', '--powermove-mcp'], { cwd: desktopDir, stdio: ['pipe', 'pipe', 'pipe'], env });
} else {
  const cli = process.env.POWERMOVE_TEST_CLI || path.join(repository, 'packages/cli/bin/powermove.mjs');
  child = spawn(process.execPath, [cli, 'mcp', '--user-data', profile, '--exports', exportsDir], { stdio: ['pipe', 'pipe', 'pipe'] });
}
let diagnostics = '', output = '', nextId = 0;
const pending = new Map();
child.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk.toString()).slice(-20000); });
child.stdout.on('data', chunk => {
  output += chunk.toString();
  for (;;) {
    const newline = output.indexOf('\n'); if (newline < 0) break;
    const line = output.slice(0, newline); output = output.slice(newline + 1);
    let message;
    try { message = JSON.parse(line); } catch { for (const call of pending.values()) call.reject(new Error(`Non-MCP stdout: ${line}`)); return; }
    const call = pending.get(message.id); if (!call) continue;
    pending.delete(message.id); clearTimeout(call.timer);
    if (message.error) call.reject(new Error(message.error.message)); else call.resolve(message.result);
  }
});
child.once('exit', code => { for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error(`MCP exited (${code}): ${diagnostics}`)); } pending.clear(); });
const rpc = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++nextId;
  const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timed out: ${method}. ${diagnostics}`)); }, 120000);
  pending.set(id, { resolve, reject, timer }); child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
});
const tool = async (name, args = {}) => {
  const result = await rpc('tools/call', { name, arguments: args });
  if (result.isError) throw new Error(`${name}: ${result.content.map(item => item.text || '').join('\n')}`);
  return result;
};
const text = result => JSON.parse(result.content.find(item => item.type === 'text').text);
const probe = (file, entries) => JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-show_entries', entries, '-of', 'json', file], { encoding: 'utf8' }));
const quit = async () => {
  child.stdin.end();
  await new Promise((resolve, reject) => { const timer = setTimeout(() => reject(new Error('MCP did not shut down')), 10000); child.once('exit', code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(diagnostics)); }); });
};

/** The desktop gate: export_video and render_frames through the job engine. */
async function desktopSmoke() {
  assert.equal((await rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } })).serverInfo.name, 'powermove');
  const tools = (await rpc('tools/list')).tools.map(tool => tool.name);
  for (const name of ['create_project', 'render_frames', 'export_video', 'get_render_status']) assert(tools.includes(name), name);
  // A cold start can answer before the hidden editor window has loaded.
  for (const until = Date.now() + 60000; ;) {
    try {
      await tool('create_project', { name: 'MCP desktop engine', width: 320, height: 180, fps: 24, duration: 1 });
      break;
    } catch (error) {
      if (!/No Powermove editor is available/.test(error.message) || Date.now() > until) throw error;
      await new Promise(resolve => setTimeout(resolve, 500));
    }
  }
  await tool('start_session', { prompt: 'Create a red video with an audible tone' });
  const imported = text(await tool('import_media', { path: audio }));
  assert.equal(imported.persisted, true);
  await tool('get_project_state');
  await tool('edit_video', { operation: 'insert', assetId: imported.assetId, from: 0, sourceIn: 0, duration: 1, label: 'Tone' });
  await tool('apply_commands', { label: 'Red background', commands: [JSON.stringify({ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 320, h: 180 }, duration: 1 })] });
  const reviewed = await tool('render_frames', { times: [0, 0.5], width: 320 });
  const images = reviewed.content.filter(item => item.type === 'image');
  assert.equal(images.length, 2);
  assert(images.every(item => item.mimeType === 'image/jpeg'), 'agent frames from the job engine are JPEG');
  assert(!/render_frames routed/.test(appLog), `render_frames was routed to the current engine: ${appLog.match(/render_frames routed[^\n]*/)?.[0]}`);
  const video = text(await tool('export_video', { name: 'MCP desktop engine', audio: true, motionBlur: false }));
  assert(video.jobId, `export_video did not run in the job engine: ${JSON.stringify(video)} ${appLog.match(/export_video routed[^\n]*/)?.[0] ?? ''}`);
  assert(video.path.startsWith(exportsDir), video.path);
  const info = await stat(video.path);
  assert(info.size > 1000);
  assert.equal(info.mode & 0o777, 0o600, 'agent exports are owner-only');
  const h264 = probe(video.path, 'stream=codec_name,profile,pix_fmt,color_primaries,color_transfer,color_space,color_range,nb_frames,sample_rate,channels');
  const v = h264.streams.find(item => item.codec_name === 'h264'), a = h264.streams.find(item => item.codec_name === 'aac');
  assert(v && a, JSON.stringify(h264));
  // CICP 1-13-1, limited range (amendment 7).
  assert.equal(v.pix_fmt, 'yuv420p'); assert.equal(v.color_primaries, 'bt709'); assert.equal(v.color_transfer, 'iec61966-2-1');
  assert.equal(v.color_space, 'bt709'); assert.equal(v.color_range, 'tv');
  assert.equal(Number(v.nb_frames), 24);
  const decodedAudio = execFileSync(ffmpeg, ['-v', 'error', '-i', video.path, '-map', '0:a:0', '-f', 's16le', '-']);
  assert(decodedAudio.length > 40000 && decodedAudio.some(byte => byte !== 0), 'MP4 audio must be present and audible');
  const pixel = execFileSync(ffmpeg, ['-v', 'error', '-i', video.path, '-frames:v', '1', '-vf', 'crop=2:2:0:0', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-']);
  assert(pixel[0] > 150 && pixel[1] < 120, `Unexpected output pixel: ${pixel.subarray(0, 3)}`);
  // Async: queued at once, then get_render_status until the job is done.
  const queued = text(await tool('export_video', { name: 'MCP desktop engine', format: 'prores', audio: false, motionBlur: false, mode: 'async' }));
  assert.equal(queued.status, 'queued');
  let status;
  for (const until = Date.now() + 120000; ;) {
    status = text(await tool('get_render_status', { jobId: queued.jobId }));
    if (['done', 'failed', 'cancelled'].includes(status.state)) break;
    assert(Date.now() < until, `the async export did not finish: ${JSON.stringify(status)}`);
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  assert.equal(status.state, 'done', JSON.stringify(status));
  assert.equal(status.progress, 1);
  assert(status.path.startsWith(exportsDir) && status.path.endsWith('.mov'), status.path);
  assert.notEqual(status.path, video.path);
  const prores = probe(status.path, 'stream=codec_name,codec_tag_string,profile,pix_fmt,nb_frames:stream_tags=timecode');
  const pv = prores.streams.find(item => item.codec_name === 'prores');
  assert(pv, JSON.stringify(prores)); assert.equal(Number(pv.nb_frames), 24);
  // MOV exports carry a tmcd track naming the first frame.
  assert(prores.streams.some(item => item.codec_tag_string === 'tmcd' && item.tags?.timecode === '00:00:00:00'), JSON.stringify(prores));
  const other = await rpc('tools/call', { name: 'get_render_status', arguments: { jobId: 'not-a-job' } });
  assert.equal(other.isError, true);
  // Capability routing: text layers are not in the Phase 2 renderer, so the
  // project goes to the current exporter and capture, and still delivers.
  await tool('get_project_state');
  await tool('apply_commands', { label: 'Title', commands: [JSON.stringify({ type: 'add_layer', layerType: 'text', name: 'Title', content: { text: 'Routed' }, duration: 1 })] });
  const routedFrames = await tool('render_frames', { times: [0.25], width: 320 });
  assert.equal(routedFrames.content.filter(item => item.type === 'image').length, 1);
  assert.match(appLog, /render_frames routed to the current engine: .*text layers/);
  const routed = text(await tool('export_video', { name: 'MCP desktop routed', audio: true, motionBlur: false }));
  assert.equal(routed.jobId, undefined, 'a routed export runs in the current exporter');
  assert.match(appLog, /export_video routed to the current engine: .*text layers/);
  assert(routed.path.startsWith(exportsDir) && (await stat(routed.path)).size > 1000, routed.path);
  assert(probe(routed.path, 'stream=codec_name').streams.some(item => item.codec_name === 'h264'));
  // A routed batch never holds the job engine: without the text layer,
  // render_frames and export_video run in the job engine again, promptly.
  const title = text(await tool('get_project_state')).layers.find(layer => layer.name === 'Title');
  assert(title, 'the Title layer');
  await tool('apply_commands', { label: 'No title', commands: [JSON.stringify({ type: 'delete_layers', targets: [title.id] })] });
  const routedSoFar = (appLog.match(/render_frames routed/g) ?? []).length;
  const again = Date.now();
  const unblocked = await tool('render_frames', { times: [0.75], width: 320 });
  assert.equal(unblocked.content.filter(item => item.type === 'image').length, 1);
  assert.equal((appLog.match(/render_frames routed/g) ?? []).length, routedSoFar, 'render_frames ran in the job engine again');
  assert(Date.now() - again < 30000, `render_frames after a routed batch took ${Date.now() - again} ms`);
  const after = text(await tool('export_video', { name: 'MCP desktop after routing', audio: false, motionBlur: false }));
  assert(after.jobId, `export_video after a routed batch did not run in the job engine: ${JSON.stringify(after)}`);
  await tool('finish_session', { commit: true });
  // No temporary or empty files are left behind.
  const left = await readdir(exportsDir);
  assert.deepEqual(left.filter(name => name.startsWith('.powermove-')), []);
  for (const name of left) assert((await stat(path.join(exportsDir, name))).size > 0, name);
  await quit();
  console.log(`MCP desktop smoke passed: render_frames and export_video (H.264 wait, ProRes async with get_render_status) through the job engine, a text project routed to the current exporter, and the job engine still serving after it; ${left.length} verified exports.`);
}

try {
  if (desktop) { await desktopSmoke(); throw Object.assign(new Error('done'), { done: true }); }
  assert.equal((await rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } })).serverInfo.name, 'powermove');
  const tools = (await rpc('tools/list')).tools.map(tool => tool.name);
  for (const name of ['create_project', 'apply_commands', 'edit_video', 'probe_media', 'render_frames', 'delegate_task', 'export_video', 'save_project']) assert(tools.includes(name), name);
  assert.equal(new Set(tools).size, tools.length, 'MCP tool names must be unique');
  const project = text(await tool('create_project', { name: 'MCP smoke video', width: 320, height: 180, fps: 12, duration: 1 }));
  await tool('start_session', { prompt: 'Create a red video with an audible tone' });
  const firstImport = text(await tool('import_media', { path: audio }));
  assert.equal(firstImport.status, 'imported'); assert.equal(firstImport.persisted, true);
  assert.equal(text(await tool('get_project_state')).layerCount, 0);
  const imported = text(await tool('import_media_to_timeline', { path: audio, at: 0 }));
  assert.equal(imported.layers[0].type, 'audio');
  await tool('get_project_state');
  const durable = text(await tool('import_media', { path: audio }));
  assert.equal(durable.persisted, true);
  assert.equal(durable.status, 'already_imported');
  assert.equal(durable.assetId, firstImport.assetId);
  assert.equal(typeof durable.assetId, 'string');
  await tool('get_project_state');
  await tool('apply_commands', { label: 'Red background', commands: [JSON.stringify({ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 320, h: 180 }, duration: 1 })] });
  const reviewed = await tool('render_frames', { times: [0, 0.5], width: 320 });
  assert.equal(reviewed.content.filter(item => item.type === 'image').length, 2);
  const state = text(await tool('get_project_state'));
  assert.equal(state.layerCount, 2);
  const video = text(await tool('export_video', { name: 'MCP smoke video', audio: true, motionBlur: false }));
  assert(video.path.startsWith(exportsDir)); assert((await stat(video.path)).size > 1000);
  const inspected = execFileSync(ffmpeg, ['-hide_banner', '-i', video.path, '-f', 'null', '-'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  assert.equal(typeof inspected, 'string');
  const decodedAudio = execFileSync(ffmpeg, ['-v', 'error', '-i', video.path, '-map', '0:a:0', '-f', 's16le', '-']);
  assert(decodedAudio.length > 40000 && decodedAudio.some(byte => byte !== 0), 'MP4 audio must be present and audible');
  const pixel = execFileSync(ffmpeg, ['-v', 'error', '-i', video.path, '-frames:v', '1', '-vf', 'crop=2:2:0:0', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-']);
  assert(pixel[0] > 150 && pixel[1] < 120, `Unexpected output pixel: ${pixel.subarray(0, 3)}`);
  const saved = text(await tool('save_project'));
  assert(saved.path.endsWith('.pmv')); assert((await stat(saved.path)).size > 1000);
  await tool('finish_session', { commit: true });
  await tool('open_project', { projectId: project.projectId });
  await tool('start_session', { prompt: 'Check rollback' });
  await tool('get_project_state');
  await tool('apply_commands', { label: 'Temporary', commands: [JSON.stringify({ type: 'add_layer', layerType: 'text', name: 'Temporary', content: { text: 'Rollback me' } })] });
  await tool('finish_session', { commit: false });
  await tool('start_session', { prompt: 'Verify', access: 'editor' });
  assert.equal(text(await tool('get_project_state')).layerCount, 2);
  const rejected = await rpc('tools/call', { name: 'apply_commands', arguments: { commands: [] } });
  assert.equal(rejected.isError, true);
  await tool('finish_session', { commit: true });
  child.stdin.end();
  await new Promise((resolve, reject) => { const timer = setTimeout(() => reject(new Error('MCP did not shut down')), 10000); child.once('exit', code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(diagnostics)); }); });
  await assert.rejects(readFile(path.join(profile, 'mcp/connection.json')));
  await assert.rejects(readFile(path.join(profile, 'mcp-host.lock')));
  console.log('MCP smoke passed: real frames, audible MP4, editable project, rollback, planning access, clean shutdown.');
} catch (error) {
  if (!error?.done) { console.error(diagnostics); if (desktop) console.error(appLog); throw error; }
}
finally {
  for (const proc of [child, app]) {
    if (!proc || proc.exitCode !== null || proc.signalCode !== null) continue;
    proc.kill('SIGTERM');
    await new Promise(resolve => {
      const timer = setTimeout(() => { proc.kill('SIGKILL'); resolve(); }, 10000);
      proc.once('exit', () => { clearTimeout(timer); resolve(); });
    });
  }
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
