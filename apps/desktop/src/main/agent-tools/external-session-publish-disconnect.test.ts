import { EventEmitter } from 'node:events';
import { mkdtemp, mkdir, writeFile, readFile, readdir, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';
import { ExternalAgentSession } from './external-session';
import * as validator from '../codex/validate-staged-extensions';
import * as workspace from '../codex/workspace';

afterEach(() => { vi.restoreAllMocks(); });

it.each([
  ...['validation', 'refresh', 'api-pack', 'prepare'].map(phase => ({ phase, ending: 'disconnect' })),
  { phase: 'validation', ending: 'crash' },
  { phase: 'refresh', ending: 'crash' },
  { phase: 'prepare', ending: 'recovery' },
  { phase: 'validation', ending: 'active' },
])('fences abandoned publication and preserves committed outcome after $ending during $phase', async ({ phase, ending }) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-publish-disconnected-'));
  const live = path.join(root, 'extensions');
  const id = 'safe-change';
  await mkdir(path.join(live, id), { recursive: true });
  await writeFile(path.join(live, id, 'manifest.json'), JSON.stringify({ id, name: 'Safe change', version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'agent' }));
  const before = 'export default "before";';
  await writeFile(path.join(live, id, 'index.ts'), before);
  const ipc = new EventEmitter();
  let crashed = false;
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => crashed,
    send: (_channel: string, request: AgentToolRequestEvent) => queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
      runId: request.runId, callId: request.callId, ok: true,
      content: [{ type: 'text', text: JSON.stringify(request.tool === '__mcp_context'
        ? { projectId: 'project-one', projectName: 'One', projectJSON: '{"id":"project-one","revision":2,"layers":[]}', home: false }
        : {}) }], changed: false, revision: 2
    })) });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: path.join(__dirname, 'mcp-server.mjs'), timeoutMs: 1000 });
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let ready!: () => void;
  const waiting = new Promise<void>(resolve => { ready = resolve; });
  const refresh = vi.fn(async () => { if (phase === 'refresh') { ready(); await pause; } });
  const pack = [{ name: 'EXTENSIONS.md', text: 'API documentation' }];
  const apiPackFiles = vi.fn(async () => pack);
  const external = new ExternalAgentSession({ bridge, getOwner: () => owner as any, userData: root,
    extensionsDir: live, apiPackFiles, attach: async () => async () => {}, refreshExtensions: refresh });
  const opened: any = await external.call('start_session', { prompt: 'Update extension' });
  await writeFile(path.join(opened.stagingDirectory, id, 'index.ts'), 'export default "after";');
  if (phase === 'api-pack') apiPackFiles.mockImplementationOnce(async () => { ready(); await pause; return pack; });
  if (phase === 'prepare') {
    const prepare = workspace.prepareAgentWorkspace;
    vi.spyOn(workspace, 'prepareAgentWorkspace').mockImplementationOnce(async (...args) => { ready(); await pause; return prepare(...args); });
  }
  const original = validator.validateStagedExtensions;
  vi.spyOn(validator, 'validateStagedExtensions').mockImplementationOnce(async (snapshot, changes) => {
    const warnings = await original(snapshot, changes);
    if (phase === 'validation') { ready(); await pause; }
    return warnings;
  });
  const publication = external.call('publish_extensions', { extensions: [{ id, action: 'updated', summary: 'Update text' }] });
  const outcome = publication.then(value => ({ value }), error => ({ error }));
  let recovered: any;
  try {
    await Promise.race([waiting, outcome.then(result => { if ('error' in result) throw result.error; throw new Error('Publication ended before lifecycle pause'); })]);
    if (ending === 'disconnect') await external.close();
    else if (ending !== 'active') {
      crashed = true;
      owner.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
      await external.call('finish_session', { commit: false });
      crashed = false;
      owner.emit('did-finish-load');
      if (ending === 'recovery') recovered = await external.call('start_session', { prompt: 'Inspect recovered project' });
    }
    resume();
    const result = await outcome;
    if (phase === 'validation' && ending !== 'active') {
      expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe(before);
      expect(result).toMatchObject({ error: expect.objectContaining({ message: expect.stringContaining(ending === 'disconnect' ? 'connection is closed' : 'session ended') }) });
      expect(refresh).not.toHaveBeenCalled();
    } else {
      expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe('export default "after";');
      if (ending === 'active') {
        expect(result).toMatchObject({ value: { changeSetId: expect.any(String), stagingDirectory: expect.any(String) } });
        const next = result as { value: { stagingDirectory: string } };
        expect(await readFile(path.join(next.value.stagingDirectory, id, 'index.ts'), 'utf8')).toBe('export default "after";');
        await expect(external.call('get_project_state', {})).resolves.toMatchObject({ ok: true });
      } else {
        expect(await readdir(path.dirname(opened.stagingDirectory)).catch(() => [])).toEqual([]);
        expect(result).toMatchObject({ value: { changeSetId: expect.any(String), stagingDirectory: null } });
      }
      expect(refresh).toHaveBeenCalledOnce();
      await expect(readFile(path.join(opened.stagingDirectory, id, 'index.ts'))).rejects.toMatchObject({ code: 'ENOENT' });
    }
    if (recovered) {
      expect(await readFile(path.join(recovered.stagingDirectory, id, 'index.ts'), 'utf8')).toBe('export default "after";');
      await expect(external.call('get_project_state', {})).resolves.toMatchObject({ ok: true });
    }
  } finally {
    resume(); await outcome;
    await external.close(); await bridge.shutdown();
    await rm(root, { recursive: true, force: true });
  }
});
