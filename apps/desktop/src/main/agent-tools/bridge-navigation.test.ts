import { EventEmitter } from 'node:events';
import { expect, it } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';

it.each([
  { isMainFrame: true, isSameDocument: false, losesDocument: true },
  { isMainFrame: false, isSameDocument: false, losesDocument: false },
  { isMainFrame: true, isSameDocument: true, losesDocument: false },
])('ends pending MCP work only for a new main document, main=$isMainFrame same=$isSameDocument', async ({ isMainFrame, isSameDocument, losesDocument }) => {
  const ipc = new EventEmitter();
  const requests: AgentToolRequestEvent[] = [];
  const respond = (request: AgentToolRequestEvent) => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
    runId: request.runId, callId: request.callId, ok: true, content: [], changed: false, revision: 1
  });
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false,
    send: (_channel: string, request: AgentToolRequestEvent) => {
      requests.push(request);
      if (request.tool === '__finish_run') queueMicrotask(() => respond(request));
    } });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: '', timeoutMs: 1000 });
  const session = await bridge.openSession({ runId: 'navigation-run', owner: owner as any, baseRevision: 0 });
  let outcome: any;
  const pending = bridge.callRenderer(session, 'apply_commands', { commands: [] }).then(value => { outcome = { value }; }, error => { outcome = { error }; });
  try {
    owner.emit('did-start-navigation', { url: 'app://powermove/index.html', isMainFrame, isSameDocument });
    await new Promise(resolve => setImmediate(resolve));
    if (losesDocument) {
      expect(outcome).toMatchObject({ error: expect.objectContaining({ message: expect.stringMatching(/document|navigat/i) }) });
      expect(outcome.error.message).toContain('apply_commands');
      expect(outcome.error.message).toContain('outcome is unknown');
      expect(session.isClosed()).toBe(true);
      await session.finish(false);
      expect(requests.map(request => request.tool)).toEqual(['apply_commands']);
    } else {
      expect(outcome).toBeUndefined();
      expect(session.isClosed()).toBe(false);
      respond(requests[0]!); await pending;
      expect(outcome).toMatchObject({ value: { ok: true } });
      await session.finish(true);
      expect(requests.at(-1)?.tool).toBe('__finish_run');
    }
    expect(owner.listenerCount('did-start-navigation')).toBe(0);
    expect(owner.listenerCount('destroyed')).toBe(0);
    expect(owner.listenerCount('render-process-gone')).toBe(0);
  } finally { await bridge.shutdown(); await pending; }
});

it('invalidates an idle run and permits a fresh run on the same WebContents after navigation', async () => {
  const ipc = new EventEmitter();
  const requests: AgentToolRequestEvent[] = [];
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false,
    send: (_channel: string, request: AgentToolRequestEvent) => {
      requests.push(request);
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok: true, content: [], changed: false, revision: 1
      }));
    } });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: '', timeoutMs: 1000 });
  try {
    const old = await bridge.openSession({ runId: 'old-idle-run', owner: owner as any, baseRevision: 0 });
    owner.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
    expect(old.isClosed()).toBe(true);
    await expect(bridge.callRenderer(old, 'get_project_state', {})).rejects.toThrow('session is closed');
    await old.finish(true);
    expect(requests).toEqual([]);
    expect(owner.listenerCount('did-start-navigation')).toBe(0);
    owner.emit('did-finish-load');
    const fresh = await bridge.openSession({ runId: 'fresh-run', owner: owner as any, baseRevision: 0 });
    await expect(bridge.callRenderer(fresh, 'get_project_state', {})).resolves.toMatchObject({ ok: true });
    await fresh.finish(true);
    expect(requests.map(request => request.runId)).toEqual(['fresh-run', 'fresh-run']);
    expect(owner.listenerCount('did-start-navigation')).toBe(0);
  } finally { await bridge.shutdown(); }
});
