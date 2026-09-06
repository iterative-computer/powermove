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
      const timer = setTimeout(() => {
        this.pending.delete(callId);
        reject(new Error(`Powermove timed out while running ${tool}.`));
      }, this.timeoutMs);
      timer.unref();
      this.pending.set(callId, { session, tool, resolve, reject, timer });
      session.owner.send(IPC.agentToolRequest, request);
    });
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
    const response = await this.callRenderer(session, request.tool, request.arguments);
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
    const response = this.parseRendererResponse(value);
    if (!response) return;
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
