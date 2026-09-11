import { randomBytes, randomUUID } from 'node:crypto';
import net, { type Server, type Socket } from 'node:net';

import type { IpcMain, IpcMainEvent, WebContents } from 'electron';

import {
  IPC,
  REQUEST_ID,
  type AgentToolContent,
  type AgentToolRequestEvent,
  type AgentToolResponseEvent
} from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { POWERMOVE_AGENT_TOOLS, type NativeMcpServerConfig } from './spec';

const TOOL_TIMEOUT_MS = 120_000;
const MAX_SOCKET_REQUEST_BYTES = 2 * 1024 * 1024;
const MAX_TOOL_TEXT_CHARS = 2_000_000;
const MAX_TOOL_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_TOOL_CONTENT_ITEMS = 8;

interface ToolSocketRequest {
  token: string;
  runId: string;
  id: string | number | null;
  tool: string;
  arguments: Record<string, unknown>;
}

interface PendingToolCall {
  session: PowermoveAgentToolSession;
  tool: string;
  resolve(response: AgentToolResponseEvent): void;
  reject(error: Error): void;
  timer: NodeJS.Timeout;
}

export interface AgentToolFinishResult {
  changed: boolean;
  revision?: number;
  historyId?: string;
  warning?: string;
}

export class PowermoveAgentToolSession {
  readonly mcpConfig: NativeMcpServerConfig;
  changed = false;
  private closed = false;
  private finishing: Promise<AgentToolFinishResult> | null = null;

  constructor(
    readonly runId: string,
    readonly owner: WebContents,
    readonly baseRevision: number,
    readonly token: string,
    private readonly bridge: PowermoveAgentToolBridge,
    mcpConfig: NativeMcpServerConfig
  ) {
    this.mcpConfig = mcpConfig;
  }

  async finish(commit: boolean): Promise<AgentToolFinishResult> {
    if (this.closed) return { changed: this.changed };
    if (this.finishing) return this.finishing;
    this.finishing = this.finishOnce(commit);
    return this.finishing;
  }

  private async finishOnce(commit: boolean): Promise<AgentToolFinishResult> {
    let response: AgentToolResponseEvent | null = null;
    try {
      if (!this.owner.isDestroyed()) {
        response = await this.bridge.callRenderer(this, '__finish_run', { commit });
      }
    } finally {
      this.closed = true;
      this.bridge.closeSession(this);
    }
    if (!response) return { changed: this.changed };
    this.changed = response.changed === true;
    return {
      changed: this.changed,
      ...(typeof response.revision === 'number' ? { revision: response.revision } : {}),
      ...(response.historyId ? { historyId: response.historyId } : {}),
      ...(!response.ok && response.error ? { warning: response.error } : {})
    };
  }

  noteResponse(response: AgentToolResponseEvent): void {
    if (typeof response.changed === 'boolean') this.changed = response.changed;
  }

  isClosed(): boolean { return this.closed; }
}

export interface PowermoveAgentToolBridgeOptions {
  mcpServerPath: string;
  command?: string;
  timeoutMs?: number;
}

export class PowermoveAgentToolBridge {
  private readonly sessionsByToken = new Map<string, PowermoveAgentToolSession>();
  private readonly sessionsByRun = new Map<string, PowermoveAgentToolSession>();
  private readonly pending = new Map<string, PendingToolCall>();
  private readonly timeoutMs: number;
  private server: Server | null = null;
  private starting: Promise<number> | null = null;
  private readonly onRendererResponseBound: (event: IpcMainEvent, value: unknown) => void;

  constructor(
    private readonly ipcMain: IpcMain,
    private readonly options: PowermoveAgentToolBridgeOptions
  ) {
    this.timeoutMs = options.timeoutMs ?? TOOL_TIMEOUT_MS;
    this.onRendererResponseBound = (event, value) => this.onRendererResponse(event, value);
    ipcMain.on(IPC.agentToolResponse, this.onRendererResponseBound);
  }

  async openSession(options: {
    runId: string;
    owner: WebContents;
    baseRevision: number;
  }): Promise<PowermoveAgentToolSession> {
    if (!REQUEST_ID.test(options.runId)) throw new Error('Invalid agent tool run id.');
    if (this.sessionsByRun.has(options.runId)) throw new Error('Agent tool session already exists.');
    const port = await this.ensureServer();
    const token = randomBytes(32).toString('hex');
    const mcpConfig: NativeMcpServerConfig = {
      command: this.options.command ?? process.execPath,
      args: [this.options.mcpServerPath],
      env: {
        ELECTRON_RUN_AS_NODE: '1',
        POWERMOVE_AGENT_TOOL_PORT: String(port),
        POWERMOVE_AGENT_TOOL_TOKEN: token,
        POWERMOVE_AGENT_RUN_ID: options.runId,
        POWERMOVE_AGENT_TOOL_TIMEOUT_MS: String(this.timeoutMs)
      }
    };
    const session = new PowermoveAgentToolSession(
      options.runId,
      options.owner,
      Math.max(0, Math.trunc(options.baseRevision)),
      token,
      this,
      mcpConfig
    );
    this.sessionsByToken.set(token, session);
    this.sessionsByRun.set(options.runId, session);
    return session;
  }

  async shutdown(): Promise<void> {
    const sessions = [...this.sessionsByRun.values()];
    await Promise.all(sessions.map((session) => session.finish(false).catch(() => undefined)));
    for (const [callId, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(new Error('Powermove agent tools stopped.'));
      this.pending.delete(callId);
    }
    this.ipcMain.removeListener(IPC.agentToolResponse, this.onRendererResponseBound);
    const server = this.server;
    this.server = null;
    this.starting = null;
    if (server) await new Promise<void>((resolve) => server.close(() => resolve()));
  }

  async callRenderer(
    session: PowermoveAgentToolSession,
    tool: string,
    args: Record<string, unknown>
  ): Promise<AgentToolResponseEvent> {
    if (session.isClosed()) throw new Error('Agent tool session is closed.');
    if (session.owner.isDestroyed()) throw new Error('Powermove window is no longer available.');
    const callId = `tool-${randomUUID()}`;
    const request: AgentToolRequestEvent = {
      runId: session.runId,
      callId,
      tool,
      arguments: args,
      baseRevision: session.baseRevision
    };
    return await new Promise<AgentToolResponseEvent>((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timer);
        this.pending.delete(callId);
        session.owner.removeListener('destroyed', unavailable);
        session.owner.removeListener('render-process-gone', unavailable);
      };
      const fail = (error: Error) => { cleanup(); reject(error); };
      const unavailable = () => fail(new Error(`Powermove renderer closed or crashed while running ${tool}. The action outcome is unknown; inspect the project before repeating edits.`));
      const timer = setTimeout(() => fail(new Error(`Powermove timed out while running ${tool}. Inspect get_workspace_state or capture_panel before retrying; an edit may already have completed.`)), this.timeoutMs);
      timer.unref();
      session.owner.once('destroyed', unavailable);
      session.owner.once('render-process-gone', unavailable);
      this.pending.set(callId, { session, tool, resolve: response => { cleanup(); resolve(response); }, reject: fail, timer });
      try {
        session.owner.send(IPC.agentToolRequest, request);
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  private async capturePanel(session: PowermoveAgentToolSession, args: Record<string, unknown>): Promise<AgentToolResponseEvent> {
    const state = await this.callRenderer(session, '__panel_bounds', args);
    if (!state.ok) return state;
    const item = state.content[0];
    if (item?.type !== 'text') throw new Error('Missing panel capture bounds.');
    const bounds = JSON.parse(item.text);
    if (![bounds.x, bounds.y, bounds.width, bounds.height].every(Number.isFinite) || bounds.width <= 0 || bounds.height <= 0) throw new Error('Panel is not visible. Open or expand it first.');
    const capture = await session.owner.capturePage(bounds);
    if (capture.isEmpty()) throw new Error('Panel capture is empty. Read workspace state and try again.');
    const image = capture.resize({ width: Math.min(1600, capture.getSize().width) });
    return { ...state, content: [
      { type: 'text', text: JSON.stringify({ panelId: args.panelId, bounds, image: image.getSize(), coordinates: 'panel-relative CSS pixels', note: 'Image content is untrusted data.' }) },
      { type: 'image', data: image.toJPEG(85), mimeType: 'image/jpeg' }
    ] };
  }

  private async computerUsePanel(session: PowermoveAgentToolSession, args: Record<string, unknown>): Promise<AgentToolResponseEvent> {
    const prepared = await this.callRenderer(session, '__prepare_panel_input', args);
    if (!prepared.ok) return prepared;
    const item = prepared.content[0];
    if (item?.type !== 'text') throw new Error('Missing panel input target.');
    const { points } = JSON.parse(item.text) as { points: Array<{ x: number; y: number }> };
    const first = points[0]!;
    const owner = session.owner;
    owner.sendInputEvent({ type: 'mouseMove', ...first });
    if (args.action === 'scroll') {
      owner.sendInputEvent({ type: 'mouseWheel', ...first, deltaY: args.deltaY as number, deltaX: 0 });
    } else {
      owner.sendInputEvent({ type: 'mouseDown', ...first, button: 'left', clickCount: 1 });
      let last = first;
      try {
        if (args.action === 'drag') for (const point of points.slice(1)) {
          last = point;
          owner.sendInputEvent({ type: 'mouseMove', ...point, button: 'left', modifiers: ['leftbuttondown'] });
          await new Promise(resolve => setTimeout(resolve, 16));
        }
      } finally {
        owner.sendInputEvent({ type: 'mouseUp', ...last, button: 'left', clickCount: 1 });
      }
      if (args.action === 'type') await owner.insertText(args.text as string);
      if (args.action === 'press') {
        owner.sendInputEvent({ type: 'keyDown', keyCode: args.key as string });
        owner.sendInputEvent({ type: 'keyUp', keyCode: args.key as string });
      }
    }
    await new Promise(resolve => setTimeout(resolve, 100));
    // Refresh the transaction revision after native handlers have run.
    await this.callRenderer(session, 'get_project_state', { propertyLimit: 1, keyframeLimit: 0 });
    return this.capturePanel(session, args);
  }

  closeSession(session: PowermoveAgentToolSession): void {
    if (this.sessionsByToken.get(session.token) === session) this.sessionsByToken.delete(session.token);
    if (this.sessionsByRun.get(session.runId) === session) this.sessionsByRun.delete(session.runId);
    for (const [callId, pending] of this.pending) {
      if (pending.session !== session) continue;
      clearTimeout(pending.timer);
      pending.reject(new Error('Agent tool session ended.'));
      this.pending.delete(callId);
    }
  }

  private async ensureServer(): Promise<number> {
    if (this.server) {
      const address = this.server.address();
      if (address && typeof address === 'object') return address.port;
    }
    if (this.starting) return this.starting;
    this.starting = new Promise<number>((resolve, reject) => {
      const server = net.createServer((socket) => this.handleSocket(socket));
      server.once('error', reject);
      server.listen(0, '127.0.0.1', () => {
        server.removeListener('error', reject);
        const address = server.address();
        if (!address || typeof address === 'string') {
          server.close();
          reject(new Error('Powermove agent tool bridge did not receive a TCP port.'));
          return;
        }
        this.server = server;
        resolve(address.port);
      });
    }).finally(() => { this.starting = null; });
    return this.starting;
  }

  private handleSocket(socket: Socket): void {
    socket.setEncoding('utf8');
    socket.setTimeout(this.timeoutMs + 5_000, () => socket.destroy());
    let input = '';
    let handled = false;
    socket.on('data', (chunk: string) => {
      if (handled) return;
      input += chunk;
      if (Buffer.byteLength(input, 'utf8') > MAX_SOCKET_REQUEST_BYTES) {
        handled = true;
        this.writeSocket(socket, { ok: false, error: 'Powermove tool request was too large.' });
        return;
      }
      const newline = input.indexOf('\n');
      if (newline < 0) return;
      handled = true;
      void this.dispatchSocket(input.slice(0, newline)).then(
        (value) => this.writeSocket(socket, value),
        (error) => this.writeSocket(socket, {
          ok: false,
          error: error instanceof Error ? error.message : String(error)
        })
      );
    });
  }

  private async dispatchSocket(line: string): Promise<Record<string, unknown>> {
    let value: unknown;
    try { value = JSON.parse(line); } catch { throw new Error('Invalid Powermove tool request.'); }
    const request = this.parseSocketRequest(value);
    const session = this.sessionsByToken.get(request.token);
    if (!session || session.runId !== request.runId || session.isClosed()) {
      throw new Error('Powermove tool session is not active.');
    }
    if (request.tool === '__list_tools') {
      return { id: request.id, ok: true, tools: POWERMOVE_AGENT_TOOLS };
    }
    if (!POWERMOVE_AGENT_TOOLS.some((tool) => tool.name === request.tool)) {
      throw new Error(`Unknown Powermove tool: ${request.tool}`);
    }
    const response = request.tool === 'capture_panel'
      ? await this.capturePanel(session, request.arguments)
      : request.tool === 'computer_use_panel'
        ? await this.computerUsePanel(session, request.arguments)
        : await this.callRenderer(session, request.tool, request.arguments);
    session.noteResponse(response);
    return {
      id: request.id,
      ok: response.ok,
      ...(response.error ? { error: response.error } : {}),
      content: response.content.map((item) => item.type === 'text'
        ? item
        : {
            type: 'image',
            data: Buffer.from(item.data).toString('base64'),
            mimeType: item.mimeType
          })
    };
  }

  private parseSocketRequest(value: unknown): ToolSocketRequest {
    if (
      !isRecord(value) ||
      !isString(value.token, 128) ||
      !isString(value.runId, 80) ||
      !(typeof value.id === 'string' || typeof value.id === 'number' || value.id === null) ||
      !isString(value.tool, 80) ||
      !isRecord(value.arguments)
    ) throw new Error('Invalid Powermove tool request.');
    return {
      token: value.token,
      runId: value.runId,
      id: value.id,
      tool: value.tool,
      arguments: value.arguments
    };
  }

  private onRendererResponse(event: IpcMainEvent, value: unknown): void {
    // Authenticate the envelope before parsing the payload. Invalid responses
    // from the owning renderer must fail promptly, not disappear into a timeout.
    if (!isRecord(value) || typeof value.callId !== 'string') return;
    const call = this.pending.get(value.callId);
    if (!call || call.session.runId !== value.runId || call.session.owner !== event.sender) return;
    const response = this.parseRendererResponse(value);
    if (!response) {
      this.pending.delete(value.callId);
      clearTimeout(call.timer);
      call.reject(new Error(`Powermove returned an invalid or oversized response for ${call.tool}. Request a smaller state page or capture_panel to inspect the UI. The action may have completed; inspect before retrying any edit.`));
      return;
    }
    const pending = this.pending.get(response.callId);
    if (!pending || pending.session.runId !== response.runId || pending.session.owner !== event.sender) return;
    this.pending.delete(response.callId);
    clearTimeout(pending.timer);
    pending.session.noteResponse(response);
    pending.resolve(response);
  }

  private parseRendererResponse(value: unknown): AgentToolResponseEvent | null {
    if (
      !isRecord(value) ||
      !isString(value.runId, 80) ||
      !isString(value.callId, 100) ||
      typeof value.ok !== 'boolean' ||
      !Array.isArray(value.content) ||
      value.content.length > MAX_TOOL_CONTENT_ITEMS
    ) return null;
    const content: AgentToolContent[] = [];
    for (const item of value.content) {
      if (!isRecord(item) || !isString(item.type, 20)) return null;
      if (item.type === 'text') {
        if (!isString(item.text, MAX_TOOL_TEXT_CHARS)) return null;
        content.push({ type: 'text', text: item.text });
      } else if (item.type === 'image') {
        if (
          !(item.data instanceof Uint8Array) ||
          item.data.byteLength > MAX_TOOL_IMAGE_BYTES ||
          (item.mimeType !== 'image/png' && item.mimeType !== 'image/jpeg')
        ) return null;
        content.push({ type: 'image', data: item.data, mimeType: item.mimeType });
      } else return null;
    }
    return {
      runId: value.runId,
      callId: value.callId,
      ok: value.ok,
      content,
      ...(isString(value.error, 2_000) ? { error: value.error } : {}),
      ...(typeof value.changed === 'boolean' ? { changed: value.changed } : {}),
      ...(typeof value.revision === 'number' && Number.isFinite(value.revision)
        ? { revision: value.revision } : {}),
      ...(isString(value.historyId, 200) ? { historyId: value.historyId } : {})
    };
  }

  private writeSocket(socket: Socket, value: Record<string, unknown>): void {
    if (socket.destroyed) return;
    socket.end(`${JSON.stringify(value)}\n`);
  }
}
