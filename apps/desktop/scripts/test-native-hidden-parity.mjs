/** A user program's desktop native export versus the native hidden host and
 * both CLI entry points (the same native pixels everywhere, no browser
 * reference), plus a byte-for-byte workspace answer versus today's Chromium
 * hidden host. Under the shared Electron lock; all profiles and synthetic
 * media stay outside the repository. */
import { spawn, execFileSync, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, readFile, writeFile, readdir, cp, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
process.env.CODEX_BINARY = path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh');
const cli = process.env.POWERMOVE_TEST_CLI ?? path.resolve(desktop, '../../packages/cli/bin/powermove.mjs');
const ffmpeg = path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffmpeg');
const root = await mkdtemp(path.join(os.tmpdir(), 'pm-native-parity-'));
const evidence = process.env.POWERMOVE_NATIVE_PARITY_EVIDENCE;
const electron = createRequire(path.join(desktop, 'package.json'))('electron');
const code = 'float a = v_st.x * u_res.x - gl_FragCoord.x; o = vec4(fract(a * 1048576.0), 0.37, 0.71, 1.0);';
const children = new Set();
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function start(file, args, options) { const child = spawn(file, args, options); children.add(child); child.once('exit', () => children.delete(child)); return child; }
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve)); child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 15_000); timer.unref(); await exited; clearTimeout(timer);
}
function mcp(child) {
  let id = 0, buffer = '', diagnostics = ''; const pending = new Map(), angleProjects = new Set();
  child.stderr.on('data', b => {
    const joined = diagnostics + b;
    for (const match of joined.matchAll(/\[native\] ANGLE runner pinned to ([A-Za-z0-9_-]+)/g)) angleProjects.add(match[1]);
    diagnostics = joined.slice(-30000);
  });
  child.stdout.on('data', b => { buffer += b; for (;;) { const at = buffer.indexOf('\n'); if (at < 0) break; const message = JSON.parse(buffer.slice(0, at)); buffer = buffer.slice(at + 1); const call = pending.get(message.id); if (call) { pending.delete(message.id); clearTimeout(call.timer); message.error ? call.reject(new Error(message.error.message)) : call.resolve(message.result); } } });
  child.once('exit', code => { for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error(`MCP exited ${code}: ${diagnostics}`)); } pending.clear(); });
  const rpc = (method, params = {}) => new Promise((resolve, reject) => { const call = ++id, timer = setTimeout(() => { pending.delete(call); reject(new Error(`${method} timed out: ${diagnostics}`)); }, 120000); pending.set(call, { resolve, reject, timer }); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: call, method, params }) + '\n'); });
  const tool = async (name, args = {}) => { const result = await rpc('tools/call', { name, arguments: args }); if (result.isError) throw new Error(`${name}: ${JSON.stringify(result)}\n${diagnostics}`); return JSON.parse(result.content.find(c => c.type === 'text').text); };
  return { child, rpc, tool, diagnostics: () => diagnostics, sawAngleRunner: project => angleProjects.has(project), initialize: () => rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'native-parity', version: '1' } }),
    async close() {
      const exited = new Promise(resolve => child.once('exit', resolve)); child.stdin.end();
      let timer;
      try { await Promise.race([exited, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('MCP did not close')), 15000); })]); }
      finally { clearTimeout(timer); }
    } };
}
for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, () => { void Promise.all([...children].map(stop)).finally(async () => { await rm(root, { recursive: true, force: true }); process.exit(130); }); });
const pixels = file => execFileSync(ffmpeg, ['-v', 'error', '-i', file, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgba', '-']);
const pngs = async folder => (await readdir(folder)).filter(n => n.endsWith('.png')).sort().map(n => path.join(folder, n));
const savedProject = async file => { const bytes = await readFile(file); return JSON.parse(bytes.subarray(9, 9 + bytes.readUInt32LE(5))).document.proj; };
let app;
try {
  const profile = path.join(root, 'desktop-profile'), exportsDir = path.join(root, 'desktop-exports');
  const appEnv = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '1', POWERMOVE_AGENT_EXPORTS: exportsDir,
    POWERMOVE_DISABLE_RUNTIME_UPDATES: '1', CODEX_BINARY: path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
    POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' }; delete appEnv.ELECTRON_RUN_AS_NODE;
  let appLog = '';
  app = start(electron, ['.'], { cwd: desktop, env: appEnv, stdio: ['ignore', 'pipe', 'pipe'] });
  for (const stream of [app.stdout, app.stderr]) stream.on('data', b => { appLog = (appLog + b).slice(-30000); });
  for (const until = Date.now() + 120000; !existsSync(path.join(profile, 'mcp/connection.json')) || !appLog.includes('the job engine is ready');) { if (app.exitCode !== null || Date.now() > until) throw new Error(appLog); await delay(100); }
  const current = mcp(start(electron, ['.', '--powermove-mcp'], { cwd: desktop, env: appEnv, stdio: ['pipe', 'pipe', 'pipe'] }));
  await current.initialize();
  let project;
  for (const until = Date.now() + 60_000; !project;) {
    try { project = await current.tool('create_project', { name: 'Host parity', width: 239, height: 137, fps: 24, duration: 0.25 }); }
    catch (error) { if (!String(error).includes('No Powermove editor is available') || Date.now() >= until) throw error; await delay(200); }
  }
  const session = await current.tool('start_session', { prompt: 'Export a user program' });
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
  const desktopExport = await current.tool('export_video', { name: 'Desktop', format: 'png', audio: false, motionBlur: false });
  const cliSource = await current.tool('save_project');
  await current.tool('set_panel_layout', { name: 'Saved host parity', docks: [
    { id: 'left', size: 281, panels: [{ id: 'assets', flex: true }] },
    { id: 'center', panels: [{ id: 'viewer', flex: true }] },
    { id: 'bottom', size: 219, panels: [{ id: 'timeline', flex: true }] },
    { id: 'right', size: 277, panels: [{ id: 'host-parity.panel', size: 183 }, { id: 'inspector', flex: true }] }
  ] });
  const desktopWorkspace = await current.tool('get_workspace_state');
  assert.equal(desktopWorkspace.layout.workspace.name, 'Saved host parity');
  const desktopPanels = desktopWorkspace.layout.registeredPanels;
  for (const id of ['viewer', 'timeline', 'inspector', 'layer-effects', 'toolbar', 'host-parity.panel']) assert(desktopPanels.some(panel => panel.id === id), `desktop panel registry is incomplete: ${id}`);
  await current.tool('finish_session', { commit: true }); await current.close(); await stop(app); app = null;
  const nativeProfile = path.join(root, 'native-profile'), legacyProfile = path.join(root, 'legacy-profile');
  await cp(profile, nativeProfile, { recursive: true, preserveTimestamps: true }); await cp(profile, legacyProfile, { recursive: true, preserveTimestamps: true });
  const open = async (profile, enabled) => {
    const client = mcp(start(process.execPath, [cli, 'mcp', '--user-data', profile, '--exports', path.join(root, `${enabled ? 'native' : 'legacy'}-exports`)],
      { env: { ...process.env, POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' }, stdio: ['pipe', 'pipe', 'pipe'] }));
    await client.initialize(); await client.tool('open_project', { projectId: project.projectId }); await client.tool('start_session', { prompt: 'Compare the persisted workspace' }); return client;
  };
  const old = await open(legacyProfile, false); const expectedWorkspace = await old.tool('get_workspace_state'); await old.tool('finish_session', { commit: true }); await old.close();
  const native = await open(nativeProfile, true);
  const restored = await native.tool('get_project_state');
  assert.equal(restored.layerCount, 1, 'the native host recovered the saved layer');
  assert.equal(restored.layers[0].id, layer.id, 'the native host retained the original layer identity');
  assert(JSON.stringify(restored.layers[0]).includes('host-parity.effect'), `the saved layer retained its user effect: ${JSON.stringify(restored.layers[0])}`);
  if (evidence) {
    await mkdir(evidence, { recursive: true });
    const saved = await native.tool('save_project');
    await writeFile(path.join(evidence, 'native-before-dom-project.json'), JSON.stringify(await savedProject(saved.path)));
  }
  const actualWorkspace = await native.tool('get_workspace_state');
  if (evidence) { await mkdir(evidence, { recursive: true }); await writeFile(path.join(evidence, 'workspace-desktop.json'), JSON.stringify(desktopWorkspace)); await writeFile(path.join(evidence, 'workspace-legacy.json'), JSON.stringify(expectedWorkspace)); await writeFile(path.join(evidence, 'workspace-native.json'), JSON.stringify(actualWorkspace)); }
  const workspaceBytesEqual = JSON.stringify(actualWorkspace) === JSON.stringify(expectedWorkspace);
  const completeRegistry = JSON.stringify(actualWorkspace.layout.registeredPanels) === JSON.stringify(desktopPanels) && JSON.stringify(expectedWorkspace.layout.registeredPanels) === JSON.stringify(desktopPanels);
  const userPanel = actualWorkspace.layout.registeredPanels.some(p => p.id === 'host-parity.panel');
  const persistedLayout = JSON.stringify(actualWorkspace.layout) === JSON.stringify(desktopWorkspace.layout) && JSON.stringify(expectedWorkspace.layout) === JSON.stringify(desktopWorkspace.layout);
  const viewportEqual = JSON.stringify(actualWorkspace.viewport) === JSON.stringify(expectedWorkspace.viewport) && actualWorkspace.viewport?.width === 1440 && actualWorkspace.viewport?.height === 1000;
  const hiddenExport = await native.tool('export_video', { name: 'Hidden', format: 'png', audio: false, motionBlur: false });
  if (evidence) {
    const saved = await native.tool('save_project');
    await writeFile(path.join(evidence, 'desktop-project.json'), JSON.stringify(await savedProject(cliSource.path)));
    await writeFile(path.join(evidence, 'native-project.json'), JSON.stringify(await savedProject(saved.path)));
  }
  const before = await pngs(desktopExport.path), after = await pngs(hiddenExport.path);
  assert.equal(before.length, 6); assert.equal(after.length, 6);
  const reference = pixels(before[0]);
  const nonBlackReference = reference.some((_, i) => i % 4 === 0 && reference[i + 1] > 0 && reference[i + 2] > 0);
  assert(nonBlackReference, 'the shader reference must distinguish a missing black composition');
  if (evidence) { await cp(desktopExport.path, path.join(evidence, 'desktop-png'), { recursive: true }); await cp(hiddenExport.path, path.join(evidence, 'hidden-png'), { recursive: true }); }
  if (evidence) {
    await writeFile(path.join(evidence, 'restored-project.json'), JSON.stringify(restored)); await writeFile(path.join(evidence, 'native.log'), native.diagnostics());
    for (const [name, directory] of [['desktop', profile], ['native', nativeProfile]]) {
      const cache = path.join(directory, 'Engine Cache');
      await cp(path.join(cache, 'definitions.json'), path.join(evidence, `${name}-definitions.json`)).catch(() => {});
    }
  }
  // A program the engine translates never needs the browser runner (E-R1).
  assert(!native.sawAngleRunner(project.projectId), `the hidden export drew a translatable program through ANGLE\n${native.diagnostics()}`);
  const differentFrames = [];
  for (let i = 0; i < Math.min(before.length, after.length); i++) if (!pixels(before[i]).equals(pixels(after[i]))) differentFrames.push(i);
  const pixelsEqual = before.length > 0 && before.length === after.length && differentFrames.length === 0;
  const result = { frames: before.length, hiddenFrames: after.length, pixelsEqual, differentFrames, workspaceBytesEqual, userPanel, completeRegistry, persistedLayout, viewportEqual, restoredLayers: restored.layerCount, angleRunner: false, nonBlackReference, panels: desktopPanels.length };
  if (evidence) await writeFile(path.join(evidence, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  await native.tool('finish_session', { commit: true }); await native.close();
  // Both public entry points draw the same native pixels as the desktop.
  const binary = process.env.POWERMOVE_PM_CLI;
  assert(binary, 'set POWERMOVE_PM_CLI to the shipping binary for the CLI gate');
  const definitions = path.join(profile, 'Engine Cache/definitions.json');
  assert(existsSync(definitions), 'desktop persists its winning definitions');
  const cliEnv = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_PM_CLI: binary,
    POWERMOVE_NODE: process.execPath };
  for (const key of ['POWERMOVE_ENGINE_SHADER_CACHE', 'POWERMOVE_ANGLE_ENDPOINT', 'POWERMOVE_ANGLE_TOKEN', 'POWERMOVE_ANGLE_HELPER']) delete cliEnv[key];
  const run = promisify(execFile);
  const cliOutputs = {};
  for (const mode of ['node', 'direct']) {
    const folder = path.join(root, `cli-${mode}`);
    const command = mode === 'node' ? process.execPath : binary;
    const args = mode === 'node'
      ? [cli, 'render', cliSource.path, folder, '--format', 'png']
      : ['render', cliSource.path, '-o', folder, '--format', 'png', '--definitions', definitions];
    const env = { ...cliEnv };
    if (mode === 'node') delete env.POWERMOVE_PM_CLI; // Exercise the bundled default too.
    const answer = await run(command, args, { env, timeout: 120000, maxBuffer: 4 * 1024 * 1024 });
    const frames = await pngs(folder); assert.equal(frames.length, before.length);
    for (let i = 0; i < frames.length; i++) assert(pixels(frames[i]).equals(pixels(before[i])), `${mode} CLI frame ${i} differs`);
    if (evidence) { await cp(folder, path.join(evidence, `cli-${mode}-png`), { recursive: true }); await writeFile(path.join(evidence, `cli-${mode}.log`), answer.stdout + answer.stderr); }
    cliOutputs[mode] = frames.length;
  }
  // pm-cli scope draws the same program: its stats match the scopes of the
  // desktop's own frame 0 within 8-bit
  // quantization (the PNG holds dithered bytes; scope measures the 32-bit
  // render), and the channel means match closely.
  const scopeOf = async (args, env = cliEnv) => JSON.parse((await run(binary, ['scope', ...args], { env, timeout: 120000, maxBuffer: 4 * 1024 * 1024 })).stdout);
  const quantum = 1 / 255 + 1e-6, statKeys = ['min', 'max', 'mean', 'p001', 'p50', 'p999'], channels = ['red', 'green', 'blue', 'luma'];
  const scopeDeviation = (a, b) => Math.max(...channels.flatMap(c => statKeys.map(k => Math.abs(a[c][k] - b[c][k]))));
  // The 8-bit export dithers, so single pixels sit a step apart while every
  // channel mean stays within a twentieth of a step of the 32-bit render.
  const meanDeviation = (a, b) => Math.max(...channels.map(c => Math.abs(a[c].mean - b[c].mean)));
  const scopeFiles = ['waveform.png', 'parade.png', 'vectorscope.png'];
  const scopeReference = await scopeOf(['--frame', before[0], '-o', path.join(root, 'scope-desktop')]);
  const scopeDir = path.join(root, 'scope-render');
  // The export's even frame size (239×137 is written at 240×138), as render draws it.
  const scopeSize = `${scopeReference.width}x${scopeReference.height}`;
  const scopeRender = await scopeOf([cliSource.path, '-o', scopeDir, '--size', scopeSize, '--definitions', definitions]);
  assert.equal(scopeRender.width, scopeReference.width); assert.equal(scopeRender.height, scopeReference.height);
  const scopeDev = scopeDeviation(scopeRender, scopeReference);
  assert(scopeDev <= quantum, `scope differs from the desktop frame by ${scopeDev}: ${JSON.stringify({ scopeRender, scopeReference })}`);
  const scopeMeanDev = meanDeviation(scopeRender, scopeReference);
  assert(scopeMeanDev <= 0.05 / 255, `scope channel means differ from the desktop frame by ${scopeMeanDev}`);
  const scopeImagesEqual = (await Promise.all(scopeFiles.map(async f => (await readFile(path.join(scopeDir, f))).equals(await readFile(path.join(root, 'scope-desktop', f)))))).every(Boolean);
  if (evidence) { await cp(scopeDir, path.join(evidence, 'scope-render'), { recursive: true }); await cp(path.join(root, 'scope-desktop'), path.join(evidence, 'scope-desktop'), { recursive: true }); }
  Object.assign(result, { scope: { deviation: scopeDev, meanDeviation: scopeMeanDev, imagesEqual: scopeImagesEqual } });
  if (evidence) await writeFile(path.join(evidence, 'result.json'), JSON.stringify(result, null, 2));
  Object.assign(result, { cliFrames: cliOutputs });
  if (evidence) await writeFile(path.join(evidence, 'result.json'), JSON.stringify(result, null, 2));
  assert(pixelsEqual, `hidden export pixels differ: ${JSON.stringify(result)}`);
  assert(workspaceBytesEqual, 'full persisted workspace answer must be byte-identical');
  assert(completeRegistry, 'every desktop built-in and user panel remains registered in both hidden hosts');
  assert(persistedLayout, 'the custom saved workspace and dock sizes survive both hidden host restarts');
  assert(viewportEqual, 'the complete hidden viewport matches');
  assert(userPanel, 'user panel remains registered');
  console.log(`Native hidden parity passed: ${before.length} user-program PNG frames pixel-identical on the hidden host and both CLI entry points, with no browser runner; complete workspace bytes equal today's hidden host, including a user panel.`);
} finally { for (const child of [...children]) await stop(child); await rm(root, { recursive: true, force: true }); }
