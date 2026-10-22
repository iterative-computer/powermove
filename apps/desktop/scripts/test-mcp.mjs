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
import { mkdtemp, mkdir, readFile, writeFile, rm, stat, readdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const desktop = process.argv.includes('--desktop');
const missingBrowser = process.argv.includes('--missing-browser');
const native = process.argv.includes('--native') || missingBrowser;
if (process.env.POWERMOVE_REQUIRE_BACKGROUND === '1') {
  const manager = execFileSync('/bin/launchctl', ['managername'], { encoding: 'utf8' }).trim();
  assert.equal(manager, 'Background', 'the native service must run outside the Aqua launch session');
  console.log(`Native MCP launch session: ${manager}`);
}

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
// The MCP contract does not use an agent provider. Isolate account discovery
// so a real provider cannot recreate its cache after the profile is removed.
process.env.CODEX_BINARY = '/usr/bin/false'; // Provider login is outside this MCP contract.
// Powermove's FFmpeg build, found the way the CLI host finds it (src/server/cli.ts hostFfmpeg).
const ffmpegDir = [process.env.PM_FFMPEG_DIR, path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out')]
  .find(dir => dir && existsSync(path.join(dir, 'bin/ffmpeg')));
const ffmpeg = process.env.POWERMOVE_FFMPEG || (ffmpegDir ? path.join(ffmpegDir, 'bin/ffmpeg') : 'ffmpeg');
const ffprobe = ffmpegDir ? path.join(ffmpegDir, 'bin/ffprobe') : 'ffprobe';
const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-video-'));
let app = null, child, appLog = '', diagnostics = '';
try {
const profile = path.join(root, 'profile'), exportsDir = path.join(root, 'exports');
const audio = path.join(root, 'tone.wav');
const sourceVideo = path.join(root, 'source.mov');
execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=blue:s=160x90:r=24', '-t', '1', '-c:v', 'prores_videotoolbox', '-profile:v', '0', sourceVideo]);
const svg = path.join(root, 'mark.svg');
await writeFile(svg, '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32"><rect x="4" y="4" width="20" height="20" fill="#e83a5e"/><path d="M30 4 L60 4 L45 28 Z" fill="#3a5ee8"/></svg>');
const frames = 48000, bytes = Buffer.alloc(44 + frames * 2);
bytes.write('RIFF', 0); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22);
bytes.writeUInt32LE(48000, 24); bytes.writeUInt32LE(96000, 28); bytes.writeUInt16LE(2, 32); bytes.writeUInt16LE(16, 34);
bytes.write('data', 36); bytes.writeUInt32LE(frames * 2, 40);
for (let i = 0; i < frames; i++) bytes.writeInt16LE(Math.round(5000 * Math.sin(i * Math.PI * 2 * 440 / 48000)), 44 + i * 2);
await writeFile(audio, bytes);
const desktopDir = path.join(repository, 'apps/desktop');
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
  child = spawn(process.execPath, [cli, 'mcp', '--user-data', profile, '--exports', exportsDir], { stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, ...(native ? { POWERMOVE_NEW_ENGINE: '1', POWERMOVE_ENGINE_SHADER_CACHE: path.join(profile, 'Engine Cache', 'Shader Cache') } : {}), ...(missingBrowser ? { POWERMOVE_CHROMIUM: path.join(root, 'missing-chromium') } : {}) } });
}
let output = '', nextId = 0;
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
/** A saved .pmv's document (today's agent save: PMV3). */
const savedDocument = async file => {
  const packed = await readFile(file);
  assert.equal(packed.subarray(0, 5).toString(), 'PMV3\n');
  return JSON.parse(packed.subarray(9, 9 + packed.readUInt32LE(5))).document;
};
/** Imported asset records by file name, for comparing the hidden hosts (POWERMOVE_MCP_EVIDENCE). */
const recordImports = async (document, mode) => {
  const assets = Object.fromEntries(Object.values(document.proj?.assets ?? document.assets ?? {}).map(({ id: _id, ...asset }) => [asset.name, asset]));
  const layers = (document.proj ?? document).layers.filter(layer => layer.d?.svgSourceAsset).map(({ id: _id, d, ...layer }) => ({ type: layer.type, name: layer.name, from: layer.from, paths: d.paths?.length, svgWidth: d.svgWidth, svgHeight: d.svgHeight, p: layer.p }));
  if (process.env.POWERMOVE_MCP_EVIDENCE) { await mkdir(process.env.POWERMOVE_MCP_EVIDENCE, { recursive: true }); await writeFile(path.join(process.env.POWERMOVE_MCP_EVIDENCE, `imports-${mode}.json`), JSON.stringify({ assets, layers }, null, 2)); }
  return { assets, layers };
};
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
  const started = text(await tool('start_session', { prompt: 'Create a red video with an audible tone' }));
  if (native) {
    await tool('get_project_state');
    assert.equal((await tool('render_frames', { times: [0], width: 160 })).content.filter(item => item.type === 'image').length, 1);
    assert(!diagnostics.includes('Chromium started on demand'), 'plain native edits and frames must not start Chromium');
    await tool('open_panel', { panelId: 'timeline' });
    assert(diagnostics.includes('Chromium started on demand'), 'the first panel call starts Chromium on demand');
  }
  const videoAsset = text(await tool('import_media', { path: sourceVideo }));
  assert.equal(videoAsset.persisted, true);
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
  if (native) {
    // Imports are today's editor import in the project tab: it decodes the media and writes its record.
    assert(diagnostics.includes('Chromium started on demand'), 'imports run today\'s editor import in the project-pinned tab');
    const listed = text(await tool('list_media'));
    assert(listed.assets.every(asset => asset.status === 'available'), `host-store imports are available to the worker: ${JSON.stringify(listed)}`);
    await tool('check_project');
    await tool('replace_media', { assetId: imported.assetId, path: audio });
    await tool('open_panel', { panelId: 'timeline' });
    const workspace = text(await tool('get_workspace_state'));
    assert(workspace.layout?.registeredPanels?.length > 0, 'native hidden workspace keeps the full production panel registry');
    await tool('open_panel', { panelId: 'viewer' });
    let shown;
    for (const until = Date.now() + 30_000; ;) {
      const capture = await tool('capture_panel', { panelId: 'viewer' });
      const image = capture.content.find(item => item.type === 'image');
      assert(image, 'native CPU viewer screenshot');
      shown = Buffer.from(image.data, 'base64');
      const center = execFileSync(ffmpeg, ['-v', 'error', '-i', 'pipe:0', '-frames:v', '1', '-vf', 'crop=2:2:(iw-2)/2:(ih-2)/2', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'], { input: shown });
      if (center[0] > 150 && center[1] < 120 && center[2] < 150) break;
      if (Date.now() > until) {
        if (process.env.POWERMOVE_MCP_EVIDENCE) { await mkdir(process.env.POWERMOVE_MCP_EVIDENCE, { recursive: true }); await writeFile(path.join(process.env.POWERMOVE_MCP_EVIDENCE, 'native-cpu-viewer.jpg'), shown); }
        throw new Error(`Native CPU viewer did not present the red composition: ${center.subarray(0, 3)}. ${diagnostics}`);
      }
      await new Promise(resolve => setTimeout(resolve, 250));
    }
  }
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
  // Text stays in the native job engine, including the host font service.
  await tool('get_project_state');
  await tool('apply_commands', { label: 'Title', commands: [JSON.stringify({ type: 'add_layer', layerType: 'text', name: 'Title', content: { text: 'Native' }, duration: 1 })] });
  const titledFrames = await tool('render_frames', { times: [0.25], width: 320 });
  assert.equal(titledFrames.content.filter(item => item.type === 'image').length, 1);
  const titled = text(await tool('export_video', { name: 'MCP desktop title', audio: true, motionBlur: false }));
  assert(titled.jobId, 'text exports must stay in the job engine');
  assert(!/render_frames routed|export_video routed/.test(appLog), 'native tools must never route to the legacy renderer');

  // Publication must settle before the next batch, with the new winning shader.
  const stage = path.join(started.stagingDirectory, 'mcp-render-smoke');
  await mkdir(stage, { recursive: true });
  await writeFile(path.join(stage, 'manifest.json'), JSON.stringify({ id: 'mcp-render-smoke', name: 'MCP render smoke', version: '1.0.0', apiVersion: 3, permissions: [], contributes: ['effects'] }));
  await writeFile(path.join(stage, 'index.ts'), `import type { PowermoveAPI } from 'powermove';
export default function activate(api: PowermoveAPI) {
  api.effects.register({ id: 'mcp-render-smoke.effect', label: 'MCP render smoke', group: 'Color', params: [], frag: 'o = texture(u_tex, v_st);' });
}
`);
  await tool('publish_extensions', { extensions: [{ id: 'mcp-render-smoke', action: 'created', summary: 'MCP render contract fixture' }] });
  const title = text(await tool('get_project_state')).layers.find(layer => layer.name === 'Title');
  assert(title, 'the Title layer');
  await tool('apply_commands', { label: 'Published effect', commands: [JSON.stringify({ type: 'add_effect', target: title.id, effect: 'mcp-render-smoke.effect' })] });
  const publishedFrames = await tool('render_frames', { times: [0.75], width: 320 });
  assert.equal(publishedFrames.content.filter(item => item.type === 'image').length, 1);
  if (native) {
    // Without `at`, import_media_to_timeline imports the asset only (placement null), as today.
    const assetOnly = text(await tool('import_media_to_timeline', { path: svg }));
    assert.equal(assetOnly.layers.length, 0, JSON.stringify(assetOnly));
    await tool('get_project_state');
    const placed = text(await tool('import_media_to_timeline', { path: svg, at: 0 }));
    assert.equal(placed.layers[0]?.type, 'shape', `an SVG with paths lands as an editable shape layer: ${JSON.stringify(placed)}`);
    await tool('get_project_state');
  }
  const saved = text(await tool('save_project'));
  if (native) {
    const { assets, layers } = await recordImports(await savedDocument(saved.path), 'native');
    assert.equal(assets['mark.svg']?.format, 'svg'); assert.equal(assets['mark.svg']?.editablePaths, 2);
    assert(assets['mark.svg']?.poster === true || assets['mark.svg']?.poster === undefined);
    assert.equal(layers.length, 1);
  }
  const packed = await readFile(saved.path);
  // Today's agent save: a PMV3 container (shared/project-container.ts).
  assert.equal(packed.subarray(0, 5).toString(), 'PMV3\n');
  const headerLength = packed.readUInt32LE(5);
  const header = JSON.parse(packed.subarray(9, 9 + headerLength));
  assert(header.media.some(record => record.id === imported.assetId && record.length > 0), 'self-contained save embeds original audio');
  assert.equal(header.media.find(record => record.id === imported.assetId).type, 'audio/wav', 'save keeps today’s media MIME fallback');
  const document = header.document;
  assert(document.proj && document.history && document.proj.layers.some(layer => layer.name === 'Title'), 'save keeps project and Undo history');
  assert.equal((await stat(saved.path)).mode & 0o777, 0o600);
  await tool('finish_session', { commit: true });
  // No temporary or empty files are left behind.
  const left = await readdir(exportsDir);
  assert.deepEqual(left.filter(name => name.startsWith('.')), []);
  for (const name of left) assert((await stat(path.join(exportsDir, name))).size > 0, name);
  await quit();
  console.log(`MCP ${native ? 'native hidden' : 'desktop'} smoke passed: render_frames and export_video (H.264 wait, ProRes async with get_render_status) through the job engine, native text, published shader plus frames, and a self-contained PMV3 (today's agent save) with Undo history; ${left.length} verified exports.`);
}

  if (missingBrowser) {
    await rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'missing-browser', version: '1' } });
    await tool('create_project', { name: 'Missing Chromium', width: 320, height: 180, fps: 24, duration: 1 });
    await tool('start_session', { prompt: 'Inspect the workspace' });
    const result = await rpc('tools/call', { name: 'get_workspace_state', arguments: {} });
    assert(result.isError && result.content.some(c => /chromium-unavailable/.test(c.text ?? '')), JSON.stringify(result));
    // Imports run today's editor import in the tab, so they name the missing browser too.
    for (const name of ['import_media', 'import_media_to_timeline']) {
      const failed = await rpc('tools/call', { name, arguments: { path: audio } });
      assert(failed.isError && failed.content.some(c => /chromium-unavailable/.test(c.text ?? '')), JSON.stringify(failed));
    }
    await tool('get_project_state');
    await tool('apply_commands', { label: 'Red', commands: [JSON.stringify({ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 320, h: 180 }, duration: 1 })] });
    assert.equal((await tool('render_frames', { times: [0], width: 160 })).content.filter(item => item.type === 'image').length, 1);
    assert(!diagnostics.includes('Chromium started on demand'));
    await tool('finish_session', { commit: true }); await quit();
    console.log('Native missing-browser case passed: document edits and frames succeed; DOM tools and imports return chromium-unavailable.');
    throw Object.assign(new Error('done'), { done: true });
  }
  if (desktop || native) { await desktopSmoke(); throw Object.assign(new Error('done'), { done: true }); }
  assert.equal((await rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } })).serverInfo.name, 'powermove');
  const tools = (await rpc('tools/list')).tools.map(tool => tool.name);
  for (const name of ['create_project', 'apply_commands', 'edit_video', 'probe_media', 'render_frames', 'delegate_task', 'export_video', 'save_project']) assert(tools.includes(name), name);
  assert.equal(new Set(tools).size, tools.length, 'MCP tool names must be unique');
  const project = text(await tool('create_project', { name: 'MCP smoke video', width: 320, height: 180, fps: 12, duration: 1 }));
  await tool('start_session', { prompt: 'Create a red video with an audible tone' });
  const videoAsset = text(await tool('import_media', { path: sourceVideo }));
  assert.equal(videoAsset.persisted, true);
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
  const assetOnly = text(await tool('import_media_to_timeline', { path: svg }));
  assert.equal(assetOnly.layers.length, 0, JSON.stringify(assetOnly));
  await tool('get_project_state');
  const placedSvg = text(await tool('import_media_to_timeline', { path: svg, at: 0 }));
  assert.equal(placedSvg.layers[0]?.type, 'shape', JSON.stringify(placedSvg));
  await tool('get_project_state');
  const saved = text(await tool('save_project'));
  assert(saved.path.endsWith('.pmv')); assert((await stat(saved.path)).size > 1000);
  await recordImports(await savedDocument(saved.path), 'legacy');
  await tool('finish_session', { commit: true });
  await tool('open_project', { projectId: project.projectId });
  await tool('start_session', { prompt: 'Check rollback' });
  await tool('get_project_state');
  await tool('apply_commands', { label: 'Temporary', commands: [JSON.stringify({ type: 'add_layer', layerType: 'text', name: 'Temporary', content: { text: 'Rollback me' } })] });
  await tool('finish_session', { commit: false });
  await tool('start_session', { prompt: 'Verify', access: 'editor' });
  assert.equal(text(await tool('get_project_state')).layerCount, 3);
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
