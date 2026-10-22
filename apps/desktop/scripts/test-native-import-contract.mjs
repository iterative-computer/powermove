/** Real desktop/native-hidden/today's-hidden import contract. Run a prebuilt
 * CLI and desktop under pm-electron-lock. Only synthetic, temporary inputs. */
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile, rm, symlink } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
process.env.CODEX_BINARY = '/usr/bin/false'; // Provider login is outside this MCP contract.
const cli = process.env.POWERMOVE_TEST_CLI ?? path.resolve(desktop, '../../packages/cli/bin/powermove.mjs');
const electron = createRequire(path.join(desktop, 'package.json'))('electron');
const { _electron } = createRequire(path.join(desktop, 'package.json'))('playwright');
const ffmpeg = path.join(process.env.PM_FFMPEG_DIR ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffmpeg');
const root = await mkdtemp(path.join(os.tmpdir(), 'pm-import-contract-'));
const timelineRegression = process.argv.includes('--timeline-regression');
const children = new Set();
let application;
async function closeApplication() {
  if (!application) return;
  // Match the repository's app fixture: a disposable imported project must
  // not leave an unattended close confirmation holding the test open.
  await application.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); }).catch(() => {});
  await application.close().catch(() => {}); application = null;
}
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function start(file, args, env) {
  const child = spawn(file, args, { cwd: desktop, env, stdio: ['pipe', 'pipe', 'pipe'] });
  children.add(child); child.once('exit', () => children.delete(child)); return child;
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve)); child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 10000); timer.unref();
  await exited; clearTimeout(timer);
}
async function cleanup() { await closeApplication(); for (const child of [...children]) await stop(child); await rm(root, { recursive: true, force: true }); }
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => { void cleanup().finally(() => process.exit(130)); });
function client(child) {
  let id = 0, buffer = '', diagnostics = ''; const pending = new Map();
  child.stderr.on('data', b => { diagnostics = (diagnostics + b).slice(-30000); });
  child.stdout.on('data', b => {
    buffer += b;
    for (;;) {
      const at = buffer.indexOf('\n'); if (at < 0) break;
      const message = JSON.parse(buffer.slice(0, at)); buffer = buffer.slice(at + 1);
      const call = pending.get(message.id); if (!call) continue;
      pending.delete(message.id); clearTimeout(call.timer);
      message.error ? call.reject(new Error(message.error.message)) : call.resolve(message.result);
    }
  });
  child.once('exit', code => { for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error(`MCP exited ${code}: ${diagnostics}`)); } pending.clear(); });
  const rpc = (method, params = {}) => new Promise((resolve, reject) => {
    const call = ++id, timer = setTimeout(() => { pending.delete(call); reject(new Error(`${method}: ${diagnostics}`)); }, 120000);
    pending.set(call, { resolve, reject, timer }); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: call, method, params }) + '\n');
  });
  return { rpc, async tool(name, args = {}) {
    const result = await rpc('tools/call', { name, arguments: args });
    assert(!result.isError, `${name}: ${JSON.stringify(result)}\n${diagnostics}`);
    return JSON.parse(result.content.find(c => c.type === 'text').text);
  }, async close() {
    const exited = new Promise(resolve => child.once('exit', resolve)); child.stdin.end();
    const timer = setTimeout(() => child.kill('SIGKILL'), 15000); timer.unref(); await exited; clearTimeout(timer);
    assert.equal(child.exitCode, 0, diagnostics);
  } };
}
async function documentOf(current) {
  const saved = await current.tool('save_project'); const bytes = await readFile(saved.path);
  assert.equal(bytes.subarray(0, 5).toString(), 'PMV3\n');
  return JSON.parse(bytes.subarray(9, 9 + bytes.readUInt32LE(5))).document.proj;
}
/** Contract stimulus only: like the document contract's clock/ID seam, freeze
 * FFmpeg's random WebM container IDs. Use the real encoder and pixel settings;
 * production conversion is unchanged. This lets contentHash stay in the byte
 * comparison. media-proxy.test.ts checks frozen/default decoded pixel equality. */
async function frozenContainerFfmpeg() {
  const directory = path.join(root, 'ffmpeg'), script = path.join(directory, 'container.mjs');
  await mkdir(path.join(directory, 'bin'), { recursive: true });
  await writeFile(script, `import { spawn } from 'node:child_process';
const args = process.argv.slice(2);
if (args.includes('libvpx-vp9') && args.includes('-lossless')) args.splice(args.length - 1, 0, '-fflags', '+bitexact');
const child = spawn(${JSON.stringify(ffmpeg)}, args, { stdio: 'inherit' });
for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => child.kill(signal));
child.on('error', error => { console.error(error); process.exit(1); });
child.on('exit', (code, signal) => process.exit(code ?? (signal === 'SIGTERM' ? 143 : 130)));
`);
  const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
  await writeFile(path.join(directory, 'bin/ffmpeg'), `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(script)} "$@"\n`, { mode: 0o700 });
  await symlink(path.join(path.dirname(ffmpeg), 'ffprobe'), path.join(directory, 'bin/ffprobe'));
  return directory;
}
// Only the metadata named by this contract; values (including absent fields)
// stay intact. Full documents are retained and compared by the Playwright gate.
function metadata(project) {
  return Object.fromEntries(Object.values(project.assets).map(asset => [asset.name,
    Object.fromEntries(['kind', 'w', 'h', 'dur', 'mime', 'svg', 'editablePaths', 'format', 'poster', 'animated', 'model', 'imageSequence', 'vertices', 'triangles', 'layerDefinition'].filter(k => Object.hasOwn(asset, k)).map(k => [k, asset[k]]))]));
}
try {
  const ffmpegDirectory = await frozenContainerFfmpeg();
  await writeFile(path.join(root, 'mark.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect x="4" y="4" width="20" height="20" fill="#e83a5e"/><path d="M30 4 L60 4 L45 28 Z" fill="#3a5ee8"/></svg>');
  await writeFile(path.join(root, 'triangle.obj'), 'o Triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n');
  execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc=size=24x16:rate=2', '-t', '1', '-loop', '0', path.join(root, 'motion.gif')]);
  execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=red:size=24x16', '-frames:v', '1', path.join(root, 'still.bmp')]);
  const transcripts = {};
  for (const mode of timelineRegression ? ['native'] : ['desktop', 'legacy', 'native']) {
    console.log(`Import contract: ${mode}`);
    const profile = path.join(root, mode), exportsDir = path.join(root, `${mode}-exports`);
    const extension = path.join(profile, 'extensions/import-contract-defaults');
    await mkdir(extension, { recursive: true });
    await writeFile(path.join(extension, 'manifest.json'), JSON.stringify({ id: 'import-contract-defaults', name: 'Import contract defaults', version: '1.0.0', apiVersion: 1, contributes: ['media'] }));
    await writeFile(path.join(extension, 'index.ts'), 'export default function activate(api) { api.media.registerImportDefaults({ anchor: { x: 0.25, y: 0.75 } }); }');
    const env = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_AGENT_EXPORTS: exportsDir,
      PM_FFMPEG_DIR: ffmpegDirectory, POWERMOVE_NEW_ENGINE: mode === 'legacy' ? '0' : '1', POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_DEVTOOLS: '0', POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
    delete env.ELECTRON_RUN_AS_NODE;
    let app, page;
    if (mode === 'desktop') {
      application = await _electron.launch({ executablePath: electron, args: ['.'], cwd: desktop, env, timeout: 120000 });
      app = application.process(); page = await application.firstWindow();
      for (const until = Date.now() + 120000; !existsSync(path.join(profile, 'mcp/connection.json'));) {
        if (app.exitCode !== null || Date.now() > until) throw new Error('Desktop MCP did not start'); await delay(100);
      }
    }
    const current = client(mode === 'desktop' ? start(electron, ['.', '--powermove-mcp'], env)
      : start(process.execPath, [cli, 'mcp', '--user-data', profile, '--exports', exportsDir], env));
    await current.rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'import-contract', version: '1' } });
    for (const until = Date.now() + 60000;;) {
      try { await current.tool('create_project', { name: 'Import contract', width: 320, height: 180, fps: 24, duration: 2 }); break; }
      catch (error) { if (!String(error).includes('No Powermove editor is available') || Date.now() > until) throw error; await delay(200); }
    }
    await current.tool('start_session', { prompt: 'Compare editor media imports' });
    if (timelineRegression) {
      const imported = await current.tool('import_media_to_timeline', { path: path.join(root, 'mark.svg') });
      assert.equal(imported.layers.length, 0, 'omitted at imports only');
      const placed = await current.tool('import_media_to_timeline', { path: path.join(root, 'mark.svg'), at: 0.5 });
      assert.equal(placed.layers[0]?.type, 'shape', 'the SVG branch creates editable paths');
      const model = await current.tool('import_media_to_timeline', { path: path.join(root, 'triangle.obj'), at: 0.75 });
      assert.equal(model.layers[0]?.type, 'extension', 'the model branch creates its extension layer');
      const document = await documentOf(current);
      assert.notEqual(document.layers.find(layer => layer.type === 'shape').p['anchor.x'].v, 0, 'the registered import defaults apply');
      await current.tool('finish_session', { commit: true }); await current.close();
      console.log('Timeline import regression passed: omitted placement, editable SVG, model branch and registered defaults.');
      continue;
    }
    const documents = [], answers = [];
    for (const [name, file, at] of [
      ['import_media', 'mark.svg'], ['import_media', 'motion.gif'],
      ['import_media_to_timeline', 'still.bmp'],
      ['import_media_to_timeline', 'mark.svg', 0.5],
      ['import_media_to_timeline', 'triangle.obj', 0.75]
    ]) {
      console.log(`${mode}: ${name} ${file}`);
      let answer;
      if (mode === 'desktop' && name === 'import_media_to_timeline') {
        // Desktop external MCP intentionally has no direct-timeline endpoint.
        // Today's editor import is its reference, with the same placement
        // options the existing hidden host passes to PM.importFiles.
        await page.evaluate(() => { const input = document.createElement('input'); input.type = 'file'; input.id = 'contract-import'; document.body.append(input); });
        try {
          await page.locator('#contract-import').setInputFiles(path.join(root, file));
          answer = await page.evaluate(async at => {
            const PM = window.PM, before = new Set(PM.proj.layers.map(l => l.id));
            // Hidden-host inputs are handed over as browser files, without an
            // Electron filesystem path. Keep the desktop reference identical
            // to that handoff while using its actual importer and MIME type.
            const files = Array.from(document.querySelector('#contract-import').files).map(file => new File([file], file.name, { type: file.type }));
            await PM.importFiles(files, { placement: at === undefined ? null : { at } });
            return { layers: PM.proj.layers.filter(l => !before.has(l.id)).map(l => ({ type: l.type })) };
          }, at);
        } finally { await page.locator('#contract-import').evaluate(input => input.remove()); }
      } else answer = await current.tool(name, { path: path.join(root, file), ...(at === undefined ? {} : { at }) });
      answers.push(answer); await current.tool('get_project_state'); documents.push(await documentOf(current));
      const listed = await current.tool('list_media');
      assert(listed.assets.every(asset => asset.status === 'available'), `${mode}: imported assets available: ${JSON.stringify(listed)}`);
    }
    const meta = metadata(documents.at(-1));
    transcripts[mode] = { documents, answers, metadata: meta };
    const evidence = process.env.POWERMOVE_IMPORT_CONTRACT_EVIDENCE;
    if (evidence) { await mkdir(evidence, { recursive: true }); await writeFile(path.join(evidence, 'imports.json'), JSON.stringify(transcripts, null, 2)); }
    assert.equal(documents[2].layers.length, 0, `${mode}: omitted at imports only`);
    assert.equal(documents[3].layers[0].type, 'shape', `${mode}: SVG stays editable`);
    assert.equal(documents[3].layers[0].from, 0.5);
    assert.notEqual(documents[3].layers[0].p['anchor.x'].v, 0, `${mode}: registered import defaults apply`);
    assert(documents[4].layers.some(layer => layer.type === 'extension' && layer.d?.data?.object?.source?.assetId), `${mode}: model uses today's extension branch`);
    assert.equal(meta['mark.svg'].format, 'svg'); assert.equal(meta['mark.svg'].editablePaths, 2);
    assert.equal(documents[3].layers[0].d.paths.length, 2);
    assert.equal(meta['motion.gif'].kind, 'video'); assert(meta['motion.gif'].imageSequence.frames > 1);
    assert.equal(meta['triangle.obj'].kind, 'model'); assert.equal(meta['triangle.obj'].triangles, 1);
    await current.tool('finish_session', { commit: true }); await current.close(); if (app) await closeApplication();
  }
  if (timelineRegression) process.exitCode = 0;
  else {
  const evidence = process.env.POWERMOVE_IMPORT_CONTRACT_EVIDENCE;
  if (evidence) { await mkdir(evidence, { recursive: true }); await writeFile(path.join(evidence, 'imports.json'), JSON.stringify(transcripts, null, 2)); }
  assert.deepEqual(transcripts.native.metadata, transcripts.desktop.metadata, 'native hidden metadata matches desktop');
  assert.deepEqual(transcripts.native.metadata, transcripts.legacy.metadata, 'native hidden metadata matches today');
  console.log('Import contract passed: SVG paths, animated GIF, converted BMP, placement defaults and model metadata on desktop and both hidden hosts.');
  }
} finally { await cleanup(); }
