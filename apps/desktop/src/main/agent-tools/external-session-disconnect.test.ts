import { EventEmitter } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge, type PowermoveAgentToolSession } from './bridge';
import { ExternalAgentSession } from './external-session';

afterEach(() => { vi.restoreAllMocks(); });

it.each([
  { tool: 'create_project', cleanupFails: false },
  { tool: 'open_project', cleanupFails: false },
  { tool: 'list_projects', cleanupFails: false },
  { tool: 'create_project', cleanupFails: true },
])('does not send $tool after external disconnect during authorization, cleanup failure=$cleanupFails', async ({ tool, cleanupFails }) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-disconnected-project-'));
  const ipc = new EventEmitter();
  const send = vi.fn((_channel: string, request: AgentToolRequestEvent) => queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
    runId: request.runId, callId: request.callId, ok: true,
    content: [{ type: 'text', text: JSON.stringify({ projectId: 'new-project' }) }], changed: false, revision: 1
  })));
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false, send });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: path.join(__dirname, 'mcp-server.mjs'), timeoutMs: 1000 });
  const external = new ExternalAgentSession({ bridge, getOwner: () => owner as any, userData: root,
    extensionsDir: path.join(root, 'extensions'), apiPackFiles: async () => [], attach: async () => async () => {} });
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let ready!: () => void;
  const authorizing = new Promise<void>(resolve => { ready = resolve; });
  const original = bridge.openSession.bind(bridge);
  let authorizedSession!: PowermoveAgentToolSession;
  vi.spyOn(bridge, 'openSession').mockImplementationOnce(async options => {
    const session = await original(options);
    authorizedSession = session;
    ready(); await pause;
    return session;
  });
  const mutation = external.call(tool, tool === 'list_projects' ? {} : { name: 'Cancelled project' });
  const outcome = mutation.then(value => ({ value }), error => ({ error }));
  try {
    await authorizing;
    await external.close();
    if (cleanupFails) {
      vi.spyOn(console, 'error').mockImplementation(() => {});
      vi.spyOn(bridge, 'callRenderer').mockRejectedValueOnce(new Error('Rollback failed'));
    }
    resume();
    expect(await outcome).toMatchObject({ error: expect.objectContaining({ message: expect.stringContaining('connection is closed') }) });
    expect(send.mock.calls.some(([, request]) => request.tool === `__mcp_${tool}`)).toBe(false);
    expect(authorizedSession.isClosed()).toBe(true);
  } finally {
    resume(); await outcome;
    await external.close(); await bridge.shutdown();
    await rm(root, { recursive: true, force: true });
  }
});
