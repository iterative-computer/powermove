/** Desktop native pinned export versus the native hidden host, plus a byte-for-
 * byte workspace answer versus today's Chromium hidden host. Under the shared
 * Electron lock; all profiles and synthetic media stay outside the repository. */
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile, readdir, cp, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const cli = process.env.POWERMOVE_TEST_CLI ?? path.resolve(desktop, '../../packages/cli/bin/powermove.mjs');
const ffmpeg = path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffmpeg');
const root = await mkdtemp(path.join(os.tmpdir(), 'pm-native-parity-'));
const evidence = process.env.POWERMOVE_NATIVE_PARITY_EVIDENCE;
const electron = createRequire(path.join(desktop, 'package.json'))('electron');
const code = 'float a = v_st.x * u_res.x - gl_FragCoord.x; o = vec4(vec3(fract(a * 1048576.0)), 1.0);';
const children = new Set();
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function start(file, args, options) { const child = spawn(file, args, options); children.add(child); child.once('exit', () => children.delete(child)); return child; }
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve)); child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 15_000); timer.unref(); await exited; clearTimeout(timer);
}
function mcp(child) {
  let id = 0, buffer = '', diagnostics = ''; const pending = new Map();
  child.stderr.on('data', b => { diagnostics = (diagnostics + b).slice(-30000); });
  child.stdout.on('data', b => { buffer += b; for (;;) { const at = buffer.indexOf('\n'); if (at < 0) break; const message = JSON.parse(buffer.slice(0, at)); buffer = buffer.slice(at + 1); const call = pending.get(message.id); if (call) { pending.delete(message.id); clearTimeout(call.timer); message.error ? call.reject(new Error(message.error.message)) : call.resolve(message.result); } } });
  child.once('exit', code => { for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error(`MCP exited ${code}: ${diagnostics}`)); } pending.clear(); });
  const rpc = (method, params = {}) => new Promise((resolve, reject) => { const call = ++id, timer = setTimeout(() => { pending.delete(call); reject(new Error(`${method} timed out: ${diagnostics}`)); }, 120000); pending.set(call, { resolve, reject, timer }); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: call, method, params }) + '\n'); });
  const tool = async (name, args = {}) => { const result = await rpc('tools/call', { name, arguments: args }); if (result.isError) throw new Error(`${name}: ${JSON.stringify(result)}\n${diagnostics}`); return JSON.parse(result.content.find(c => c.type === 'text').text); };
  return { child, rpc, tool, initialize: () => rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'native-parity', version: '1' } }),
    async close() {
      const exited = new Promise(resolve => child.once('exit', resolve)); child.stdin.end();
      let timer;
      try { await Promise.race([exited, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('MCP did not close')), 15000); })]); }
      finally { clearTimeout(timer); }
    } };
}
for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, () => { void Promise.all([...children].map(stop)).finally(() => process.exit(130)); });
const pixels = file => execFileSync(ffmpeg, ['-v', 'error', '-i', file, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgba', '-']);
const pngs = async folder => (await readdir(folder)).filter(n => n.endsWith('.png')).sort().map(n => path.join(folder, n));
let app;
try {
  const profile = path.join(root, 'desktop-profile'), exportsDir = path.join(root, 'desktop-exports');
  const appEnv = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '1', POWERMOVE_AGENT_EXPORTS: exportsDir, POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' }; delete appEnv.ELECTRON_RUN_AS_NODE;
  let appLog = '';
  app = start(electron, ['.'], { cwd: desktop, env: appEnv, stdio: ['ignore', 'pipe', 'pipe'] });
  for (const stream of [app.stdout, app.stderr]) stream.on('data', b => { appLog = (appLog + b).slice(-30000); });
  for (const until = Date.now() + 120000; !existsSync(path.join(profile, 'mcp/connection.json')) || !appLog.includes('the job engine is ready');) { if (app.exitCode !== null || Date.now() > until) throw new Error(appLog); await delay(100); }
  const current = mcp(start(electron, ['.', '--powermove-mcp'], { cwd: desktop, env: appEnv, stdio: ['pipe', 'pipe', 'pipe'] }));
  await current.initialize();
  let project;
  for (const until = Date.now() + 60_000; !project;) {
    try { project = await current.tool('create_project', { name: 'Pinned host parity', width: 239, height: 137, fps: 24, duration: 0.25 }); }
    catch (error) { if (!String(error).includes('No Powermove editor is available') || Date.now() >= until) throw error; await delay(200); }
  }
  const session = await current.tool('start_session', { prompt: 'Probe and export a pinned user program' });
  const stage = path.join(session.stagingDirectory, 'host-parity'); await mkdir(stage, { recursive: true });
  await writeFile(path.join(stage, 'manifest.json'), JSON.stringify({ id: 'host-parity', name: 'Host parity', version: '1.0.0', apiVersion: 3, permissions: [], contributes: ['effects', 'panels'] }));
  await writeFile(path.join(stage, 'index.ts'), `export default function activate(api) {
    api.effects.register({ id: 'host-parity.effect', label: 'Host parity', group: 'Test', params: [], frag: ${JSON.stringify(code)} });
    api.panels.register({ id: 'host-parity.panel', title: 'Host parity', size: 180, build(body) { body.textContent = 'Host parity panel'; } });
  }`);
  await current.tool('publish_extensions', { extensions: [{ id: 'host-parity', action: 'created', summary: 'Synthetic host parity fixture' }] });
  await current.tool('apply_commands', { commands: [JSON.stringify({ type: 'add_layer', layerType: 'solid', name: 'Carrier', content: { color: '#ffffff', w: 239, h: 137 }, duration: 0.25 })] });
  const layer = (await current.tool('get_project_state')).layers.find(l => l.name === 'Carrier');
  await current.tool('apply_commands', { commands: [JSON.stringify({ type: 'add_effect', target: layer.id, effect: 'host-parity.effect' })] });
  const desktopExport = await current.tool('export_video', { name: 'Pinned desktop', format: 'png', audio: false, motionBlur: false });
  const desktopWorkspace = await current.tool('get_workspace_state');
  const desktopPanels = desktopWorkspace.layout.registeredPanels;
  for (const id of ['viewer', 'timeline', 'inspector', 'layer-effects', 'toolbar', 'host-parity.panel']) assert(desktopPanels.some(panel => panel.id === id), `desktop panel registry is incomplete: ${id}`);
  const pins = path.join(profile, 'Engine Cache/Shader Cache/angle-parity');
  const markers = await readdir(pins);
  assert(markers.some(name => name.endsWith('.runner')), 'the real desktop probe must persist a runner pin');
  assert(existsSync(path.join(profile, 'Engine Cache/Shader Cache/desktop-environment.json')), 'desktop persisted its probe namespace');
  await current.tool('finish_session', { commit: true }); await current.close(); await stop(app); app = null;
  const nativeProfile = path.join(root, 'native-profile'), legacyProfile = path.join(root, 'legacy-profile');
  await cp(profile, nativeProfile, { recursive: true, preserveTimestamps: true }); await cp(profile, legacyProfile, { recursive: true, preserveTimestamps: true });
  const open = async (profile, enabled) => {
    const client = mcp(start(process.execPath, [cli, 'mcp', '--user-data', profile, '--exports', path.join(root, `${enabled ? 'native' : 'legacy'}-exports`)],
      { env: { ...process.env, POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' }, stdio: ['pipe', 'pipe', 'pipe'] }));
    await client.initialize(); await client.tool('open_project', { projectId: project.projectId }); await client.tool('start_session', { prompt: 'Compare the persisted workspace' }); return client;
  };
  const old = await open(legacyProfile, false); const expectedWorkspace = await old.tool('get_workspace_state'); await old.tool('finish_session', { commit: true }); await old.close();
  const native = await open(nativeProfile, true); const actualWorkspace = await native.tool('get_workspace_state');
  if (evidence) { await mkdir(evidence, { recursive: true }); await writeFile(path.join(evidence, 'workspace-desktop.json'), JSON.stringify(desktopWorkspace)); await writeFile(path.join(evidence, 'workspace-legacy.json'), JSON.stringify(expectedWorkspace)); await writeFile(path.join(evidence, 'workspace-native.json'), JSON.stringify(actualWorkspace)); }
  const workspaceBytesEqual = JSON.stringify(actualWorkspace) === JSON.stringify(expectedWorkspace);
  const completeRegistry = JSON.stringify(actualWorkspace.layout.registeredPanels) === JSON.stringify(desktopPanels) && JSON.stringify(expectedWorkspace.layout.registeredPanels) === JSON.stringify(desktopPanels);
  const userPanel = actualWorkspace.layout.registeredPanels.some(p => p.id === 'host-parity.panel');
  const hiddenExport = await native.tool('export_video', { name: 'Pinned hidden', format: 'png', audio: false, motionBlur: false });
  const before = await pngs(desktopExport.path), after = await pngs(hiddenExport.path);
  if (evidence) { await cp(desktopExport.path, path.join(evidence, 'desktop-png'), { recursive: true }); await cp(hiddenExport.path, path.join(evidence, 'hidden-png'), { recursive: true }); }
  const differentFrames = [];
  for (let i = 0; i < Math.min(before.length, after.length); i++) if (!pixels(before[i]).equals(pixels(after[i]))) differentFrames.push(i);
  const pixelsEqual = before.length > 0 && before.length === after.length && differentFrames.length === 0;
  const result = { frames: before.length, hiddenFrames: after.length, pixelsEqual, differentFrames, workspaceBytesEqual, userPanel, completeRegistry, panels: desktopPanels.length, pinnedPrograms: markers.filter(n => n.endsWith('.runner')).length };
  if (evidence) await writeFile(path.join(evidence, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  await native.tool('finish_session', { commit: true }); await native.close();
  assert(pixelsEqual, `pinned export pixels differ: ${JSON.stringify(result)}`);
  assert(workspaceBytesEqual, 'full persisted workspace answer must be byte-identical');
  assert(completeRegistry, 'every desktop built-in and user panel remains registered in both hidden hosts');
  assert(userPanel, 'user panel remains registered');
  console.log(`Native hidden parity passed: ${before.length} pinned-program PNG frames pixel-identical to the desktop; complete workspace bytes equal today's hidden host, including a user panel.`);
} finally { for (const child of [...children]) await stop(child); await rm(root, { recursive: true, force: true }); }
