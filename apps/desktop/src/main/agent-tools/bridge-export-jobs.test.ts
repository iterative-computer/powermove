import { EventEmitter } from 'node:events';
import { afterEach, describe, expect, it } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';
import { PowermoveAgentToolBridge } from './bridge';

class FakeIpcMain extends EventEmitter {}

/** A renderer whose export finishes only when the test says so. */
class ExportingWebContents extends EventEmitter {
  readonly id = 7;
  readonly requests: AgentToolRequestEvent[] = [];
  private exports: AgentToolRequestEvent[] = [];
  constructor(private readonly ipc: FakeIpcMain) { super(); }
  isDestroyed() { return false; }
  isCrashed() { return false; }
  send(_channel: string, request: AgentToolRequestEvent) {
    this.requests.push(request);
    if (request.tool === '__mcp_export_video') { this.exports.push(request); return; }
    queueMicrotask(() => this.reply(request, { tool: request.tool }));
  }
  reply(request: AgentToolRequestEvent, value: unknown, ok = true) {
    this.ipc.emit(IPC.agentToolResponse, { sender: this }, {
      runId: request.runId, callId: request.callId, ok, content: [{ type: 'text', text: JSON.stringify(value) }],
      ...(ok ? {} : { error: String(value) }), changed: false, revision: 1,
    });
  }
  finishExport(value: unknown, ok = true) { this.reply(this.exports.shift()!, value, ok); }
}

const text = (response: { content: Array<{ type: string; text?: string }> }) => JSON.parse(response.content[0]!.text!);

describe('export_video jobs', () => {
  const bridges: PowermoveAgentToolBridge[] = [];
  afterEach(async () => { for (const bridge of bridges.splice(0)) await bridge.shutdown(); });
  function setup(progress: { frames: number; total?: number; seconds: number } | null = { frames: 300, total: 1200, seconds: 60 }) {
    const ipc = new FakeIpcMain(), owner = new ExportingWebContents(ipc);
    const bridge = new PowermoveAgentToolBridge(ipc as never, { mcpServerPath: '', exportWaitMs: 20, exportProgress: () => progress && { ...progress, encoder: 'h264_videotoolbox' } });
    bridges.push(bridge);
    return { owner, bridge };
  }

  it('returns the file directly when the export finishes within one call', async () => {
    const { owner, bridge } = setup();
    const session = await bridge.openSession({ runId: 'export-quick', owner: owner as never, baseRevision: 0 });
    const call = bridge.callTool(session, 'export_video', { audio: true });
    await new Promise(resolve => setTimeout(resolve, 0));
    owner.finishExport({ path: '/exports/quick.mp4' });
    expect(text(await call)).toEqual({ path: '/exports/quick.mp4' });
    expect(owner.requests.at(-1)?.arguments).toEqual({ audio: true });
  });

  it('keeps a long export running past the wait and resumes it by jobId', async () => {
    const { owner, bridge } = setup();
    const session = await bridge.openSession({ runId: 'export-long', owner: owner as never, baseRevision: 0 });
    const running = text(await bridge.callTool(session, 'export_video', {}));
    expect(running).toMatchObject({ status: 'running', jobId: expect.stringMatching(/^export-/),
      progress: { frames: 300, totalFrames: 1200, encoder: 'h264_videotoolbox', secondsRemaining: 180 } });
    expect(running.next).toContain(running.jobId);
    // Only one export per window; the error names the job to wait for.
    await expect(bridge.callTool(session, 'export_video', {})).rejects.toThrow(running.jobId);
    await expect(bridge.callTool(session, 'export_video', { jobId: running.jobId, audio: false })).rejects.toThrow('no other arguments');
    expect(text(await bridge.callTool(session, 'export_video', { jobId: running.jobId }))).toMatchObject({ status: 'running', jobId: running.jobId });
    // The export finishes while nobody waits; the next resume collects it once.
    owner.finishExport({ path: '/exports/long.mp4', timings: { fps: 24 } });
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(text(await bridge.callTool(session, 'export_video', { jobId: running.jobId }))).toEqual({ path: '/exports/long.mp4', timings: { fps: 24 } });
    await expect(bridge.callTool(session, 'export_video', { jobId: running.jobId })).rejects.toThrow('Unknown export job');
    expect(owner.requests.filter(request => request.tool === '__mcp_export_video')).toHaveLength(1);
  });

  it('reports a failure that happened while nobody was waiting, then allows a new export', async () => {
    const { owner, bridge } = setup(null);
    const session = await bridge.openSession({ runId: 'export-failing', owner: owner as never, baseRevision: 0 });
    const running = text(await bridge.callTool(session, 'export_video', {}));
    expect(running.progress).toBeUndefined();
    owner.finishExport('Video encoder stopped (while encoding)', false);
    await new Promise(resolve => setTimeout(resolve, 0));
    const resumed = await bridge.callTool(session, 'export_video', { jobId: running.jobId });
    expect(resumed.ok).toBe(false);
    expect(resumed.error).toContain('while encoding');
    const next = bridge.callTool(session, 'export_video', {});
    await new Promise(resolve => setTimeout(resolve, 0));
    owner.finishExport({ path: '/exports/retry.mp4' });
    expect(text(await next)).toEqual({ path: '/exports/retry.mp4' });
  });
});
