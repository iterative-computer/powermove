import { EventEmitter } from 'node:events';
import { expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';

async function fixture() {
  const ipc = new EventEmitter();
  let generation = 1, enabled = true;
  const requests: AgentToolRequestEvent[] = [], input: string[] = [];
  const owner = Object.assign(new EventEmitter(), {
    id: 1, isDestroyed: () => false, isCrashed: () => false,
    send(channel: string, request: AgentToolRequestEvent) {
      if (channel !== IPC.agentToolRequest) return;
      requests.push(request);
      const available = enabled && (request.tool !== '__validate_panel_target' || request.arguments.target === `generation-${generation}`);
      const value = request.tool === '__panel_bounds' ? { x: 0, y: 0, width: 100, height: 100, target: `generation-${generation}` }
        : request.tool === '__prepare_panel_input' ? { points: request.arguments.points, target: `generation-${generation}` }
        : request.tool === 'get_workspace_state' ? { panels: enabled ? ['extension-panel'] : [], generation }
        : {};
      const ok = !request.tool.startsWith('__panel') && request.tool !== '__prepare_panel_input' && request.tool !== '__validate_panel_target' || available;
      queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
        runId: request.runId, callId: request.callId, ok,
        ...(ok ? {} : { error: 'Panel is unavailable' }),
        content: [{ type: 'text', text: JSON.stringify(value) }], revision: 4
      }));
    },
    sendInputEvent(event: { type: string }) { input.push(event.type); },
    async insertText() { input.push('text'); }
  });
  let releaseCapture!: () => void;
  const captureGate = new Promise<void>(resolve => { releaseCapture = resolve; });
  const capturePanel = vi.fn(async () => { await captureGate; return Buffer.from('image'); });
  const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '', capturePanel, timeoutMs: 1000,
    userInput: { drive: () => () => {} } });
  const session = await bridge.openSession({ runId: 'panel-lifecycle', owner: owner as never, baseRevision: 4 });
  return { bridge, session, owner, requests, input, capturePanel, releaseCapture,
    disable() { enabled = false; generation++; }, enable() { enabled = true; generation++; } };
}

it('rejects a late capture after disable/re-enable and keeps the agent session usable', async () => {
  const f = await fixture();
  try {
    const captured = f.bridge.callTool(f.session, 'capture_panel', { panelId: 'extension-panel' });
    const outcome = captured.catch(error => error);
    await vi.waitFor(() => expect(f.capturePanel).toHaveBeenCalledOnce());
    f.disable(); f.enable(); f.releaseCapture();
    expect(await outcome).toMatchObject({ message: expect.stringContaining('panel changed') });
    for (let toggle = 0; toggle < 3; toggle++) {
      f.disable();
      const state = await f.bridge.callTool(f.session, 'get_workspace_state', {});
      expect(JSON.parse((state.content[0] as { text: string }).text).panels).toEqual([]);
      f.enable();
      expect((await f.bridge.callTool(f.session, 'capture_panel', { panelId: 'extension-panel' })).ok).toBe(true);
    }
    expect(f.session.isClosed()).toBe(false);
    await f.session.finish(true);
    expect(f.requests.at(-1)?.arguments).toEqual({ commit: true });
  } finally { f.releaseCapture(); await f.bridge.shutdown(); }
});

it('stops a removed panel drag, releases the mouse, and does not replay input', async () => {
  const f = await fixture();
  f.owner.sendInputEvent = event => { f.input.push(event.type); if (event.type === 'mouseDown') f.disable(); };
  try {
    await expect(f.bridge.callTool(f.session, 'computer_use_panel', { panelId: 'extension-panel', action: 'drag',
      points: [{ x: 5, y: 6 }, { x: 10, y: 10 }, { x: 15, y: 15 }] })).rejects.toThrow('do not repeat');
    expect(f.input).toEqual(['mouseMove', 'mouseDown', 'mouseUp']);
    expect(f.session.isClosed()).toBe(false);
    expect((await f.bridge.callTool(f.session, 'get_workspace_state', {})).ok).toBe(true);
  } finally { f.releaseCapture(); await f.bridge.shutdown(); }
});

it('discards an in-flight capture after cancellation and rejects later calls', async () => {
  const f = await fixture();
  try {
    const outcome = f.bridge.callTool(f.session, 'capture_panel', { panelId: 'extension-panel' }).catch(error => error);
    await vi.waitFor(() => expect(f.capturePanel).toHaveBeenCalledOnce());
    await f.session.finish(false);
    const sent = f.requests.length;
    f.releaseCapture();
    expect(await outcome).toMatchObject({ message: expect.stringContaining('closed') });
    expect(f.requests).toHaveLength(sent);
    await expect(f.bridge.callTool(f.session, 'get_workspace_state', {})).rejects.toThrow('not active');
  } finally { f.releaseCapture(); await f.bridge.shutdown(); }
});

it('rejects new calls while the run is finishing, before its renderer response arrives', async () => {
  const f = await fixture();
  const send = f.owner.send.bind(f.owner);
  let releaseFinish!: () => void;
  f.owner.send = (channel, request) => {
    if (request.tool === '__finish_run') releaseFinish = () => send(channel, request);
    else send(channel, request);
  };
  try {
    const finishing = f.session.finish(true);
    expect(f.session.isFinishing()).toBe(true);
    await expect(f.bridge.callTool(f.session, 'get_workspace_state', {})).rejects.toThrow('not active');
    expect(f.requests).toHaveLength(0);
    releaseFinish();
    await finishing;
    expect(f.session.isClosed()).toBe(true);
    expect(f.requests.map(request => request.tool)).toEqual(['__finish_run']);
  } finally { f.releaseCapture(); await f.bridge.shutdown(); }
});
