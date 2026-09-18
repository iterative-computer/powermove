import { afterEach, expect, it } from 'vitest';
import { mkdtemp, readFile, rm, symlink, access } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { CompatibleWorkspace, runWorkspaceCommand } from './compatible-workspace';
import { prepareAgentWorkspace } from './codex/workspace';
import { agentResultSchema } from './codex/instructions';

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map(dir => rm(dir, { recursive: true, force: true }))); });
const signal = () => new AbortController().signal;
async function workspace() {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-workspace-')); directories.push(directory);
  const layout = await prepareAgentWorkspace({ projectId: 'proof', provider: 'compatible', projectJSON: '{}', images: [], attachments: [] } as any, directory, 'project', agentResultSchema(), { extensionsDir: path.join(directory, 'extensions'), apiPackFiles: [] });
  return new CompatibleWorkspace(layout, 'project');
}

it('creates, reads and lists files and rejects path and symbolic-link escapes', async () => {
  const ws = await workspace();
  await ws.call('write_file', { path: 'nested/file.txt', text: 'hello' }, signal());
  expect(await readFile(path.join(ws.layout.root, 'nested/file.txt'), 'utf8')).toBe('hello');
  expect(JSON.stringify(await ws.call('read_file', { path: 'nested/file.txt', offset: 1, limit: 2 }, signal()))).toContain('el');
  expect(JSON.stringify(await ws.call('list_files', { path: 'nested' }, signal()))).toContain('file.txt');
  await expect(ws.call('write_file', { path: '../escaped.txt', text: 'no' }, signal())).rejects.toThrow('outside');
  await symlink(path.dirname(ws.layout.root), path.join(ws.layout.root, 'escape'));
  await expect(ws.call('write_file', { path: 'escape/escaped.txt', text: 'no' }, signal())).rejects.toThrow('symbolic link');
});

it('reports real compile errors before publishing and exports created artifacts', async () => {
  const ws = await workspace();
  const dir = path.join(ws.layout.stagingDirectory, 'broken-effect');
  await ws.call('write_file', { path: path.join(dir, 'manifest.json'), text: JSON.stringify({ id: 'broken-effect', name: 'Broken', version: '1.0.0', apiVersion: 1, contributes: ['effects'] }) }, signal());
  await ws.call('write_file', { path: path.join(dir, 'index.ts'), text: 'export const = ;' }, signal());
  const result = { summary: 'Done', commands: [], artifacts: [], extensions: [{ id: 'broken-effect', action: 'created' }], notes: [], externalActions: [] };
  await expect(ws.finish(result, signal())).rejects.toThrow('failed compilation');
  await expect(access(path.join(ws.layout.liveDirectory, 'broken-effect'))).rejects.toThrow();
  await ws.call('write_file', { path: path.join(dir, 'index.ts'), text: 'export default function activate() {}' }, signal());
  await ws.call('write_file', { path: path.join(ws.layout.runDirectory, 'note.txt'), text: 'proof' }, signal());
  const finished = await ws.finish(result, signal());
  expect(finished.ok).toBe(true);
  if (finished.ok) expect(JSON.parse(finished.text).artifacts).toEqual([expect.objectContaining({ name: 'note.txt', size: 5 })]);
});

it.runIf(process.platform === 'darwin')('runs commands with real Project write isolation and propagates cancellation', async () => {
  const ws = await workspace();
  const outside = path.join(path.dirname(ws.layout.root), 'outside.txt');
  const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
  const success = await runWorkspaceCommand(ws.layout.root, 'project', 'printf proof > proof.txt && cat proof.txt', 5000, signal());
  expect(success).toMatchObject({ output: 'proof', exitCode: 0 });
  const denied = await runWorkspaceCommand(ws.layout.root, 'project', `printf denied > ${quote(outside)}`, 5000, signal());
  expect(denied.exitCode).not.toBe(0);
  await expect(access(outside)).rejects.toThrow();
  await expect(runWorkspaceCommand(ws.layout.root, 'project', 'sleep 10', 25, signal())).rejects.toThrow('timed out');
  const controller = new AbortController();
  const pending = runWorkspaceCommand(ws.layout.root, 'project', 'sleep 10', 5000, controller.signal);
  setTimeout(() => controller.abort(new Error('Stopped by test')), 30);
  await expect(pending).rejects.toThrow('Stopped by test');
});
