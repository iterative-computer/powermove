import { EventEmitter } from 'node:events';
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';
import { ExternalAgentSession } from './external-session';

afterEach(() => { vi.restoreAllMocks(); });

it.each([
  { isMainFrame: true, isSameDocument: false, losesDocument: true },
  { isMainFrame: false, isSameDocument: false, losesDocument: false },
  { isMainFrame: true, isSameDocument: true, losesDocument: false },
])('releases an external transaction only when its main document changes, main=$isMainFrame same=$isSameDocument', async ({ isMainFrame, isSameDocument, losesDocument }) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-navigation-'));
  const ipc = new EventEmitter();
  let document = 1;
  const requests: Array<{ document: number; request: AgentToolRequestEvent }> = [];
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false,
    send: (_channel: string, request: AgentToolRequestEvent) => {
      requests.push({ document, request });
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok: true,
        content: [{ type: 'text', text: JSON.stringify(request.tool === '__mcp_context'
          ? { projectId: `project-${document}`, projectName: 'One', projectJSON: '{"id":"project-one","revision":2,"layers":[]}', home: false } : {}) }], changed: false, revision: 2
      }));
    } });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: '', timeoutMs: 1000 });
  const detach = vi.fn(async () => {});
  const external = new ExternalAgentSession({ bridge, getOwner: () => owner as any, userData: root,
    extensionsDir: path.join(root, 'extensions'), apiPackFiles: async () => [], attach: async () => detach });
  try {
    const opened: any = await external.call('start_session', { prompt: 'Inspect', access: losesDocument ? 'project' : 'editor' });
    if (losesDocument) await writeFile(path.join(opened.artifactsDirectory, 'partial.txt'), 'unfinished');
    if (losesDocument) document = 2;
    owner.emit('did-start-navigation', { url: 'app://powermove/index.html', isMainFrame, isSameDocument });
    await new Promise(resolve => setImmediate(resolve));
    await new Promise(resolve => setImmediate(resolve));
    if (losesDocument) {
      expect(detach).toHaveBeenCalledOnce();
      expect(requests.some(entry => entry.document === 2 && entry.request.tool === '__finish_run')).toBe(false);
      await vi.waitFor(async () => expect(await stat(opened.stagingDirectory).catch(() => null)).toBeNull());
      expect(await stat(opened.artifactsDirectory).catch(() => null)).toBeNull();
      await expect(external.call('get_project_state', {})).rejects.toThrow('start_session');
      owner.emit('did-finish-load');
      await expect(external.call('start_session', { prompt: 'Inspect recovered page', access: 'editor' })).resolves.toMatchObject({ projectId: 'project-2' });
    } else {
      expect(detach).not.toHaveBeenCalled();
      await expect(external.call('get_project_state', {})).resolves.toMatchObject({ ok: true });
    }
  } finally {
    await external.close(); await bridge.shutdown();
    expect(owner.listenerCount('did-start-navigation')).toBe(0);
    expect(owner.listenerCount('destroyed')).toBe(0);
    expect(owner.listenerCount('render-process-gone')).toBe(0);
    await rm(root, { recursive: true, force: true });
  }
});
