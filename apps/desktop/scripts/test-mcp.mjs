/** Real MCP → hidden editor → media import → frames → MP4/project smoke test.
 * Build the CLI first. All projects/media/exports stay in an isolated temp profile. */
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile, rm, stat } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const require = createRequire(path.join(repository, 'apps/desktop/package.json'));
const ffmpeg = require('ffmpeg-static');
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
const cli = process.env.POWERMOVE_TEST_CLI || path.join(repository, 'packages/cli/bin/powermove.mjs');
const child = spawn(process.execPath, [cli, 'mcp', '--user-data', profile, '--exports', exportsDir], { stdio: ['pipe', 'pipe', 'pipe'] });
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
try {
  assert.equal((await rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } })).serverInfo.name, 'powermove');
  const tools = (await rpc('tools/list')).tools.map(tool => tool.name);
  for (const name of ['create_project', 'apply_commands', 'edit_video', 'probe_media', 'render_frames', 'delegate_task', 'export_video', 'save_project']) assert(tools.includes(name), name);
  const project = text(await tool('create_project', { name: 'MCP smoke video', width: 320, height: 180, fps: 12, duration: 1 }));
  await tool('start_session', { prompt: 'Create a red video with an audible tone' });
  const imported = text(await tool('import_media', { path: audio, at: 0 }));
  assert.equal(imported.layers[0].type, 'audio');
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
} catch (error) { console.error(diagnostics); throw error; }
finally { child.kill('SIGTERM'); await rm(root, { recursive: true, force: true }); }
