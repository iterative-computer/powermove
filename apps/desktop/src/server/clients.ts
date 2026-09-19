/*
 * The remote host's stand-ins for Electron's IpcMain, WebContents and
 * BrowserWindow. Every main-process module registers against the structural
 * `Pick<IpcMain, 'handle' | 'on'>` surface and talks back through
 * `event.sender.send(...)`, so one WebSocket connection can play the part of
 * one editor window without those modules changing.
 */
import { EventEmitter } from 'node:events';

import { decodeFrame, encodeFrame, isClientMessage, type ClientMessage, type ServerMessage } from '../shared/wire';

export interface Socket {
  send(data: Uint8Array): void;
  close(code?: number, reason?: string): void;
}

type InvokeHandler = (event: RemoteEvent, ...args: unknown[]) => unknown;
type Listener = (event: RemoteEvent, ...args: unknown[]) => void;

/** Lets the host stand a different sender in front of main's handlers (run
 *  owners for agent runs) and observe results. */
export interface ChannelInterceptor {
  sender?(client: RemoteClient, args: unknown[]): { id: number; isDestroyed(): boolean; send(channel: string, ...args: unknown[]): void } | null;
  after?(client: RemoteClient, args: unknown[], value: unknown, error: unknown): void;
}

/** What handlers see. `returnValue` is the sendSync reply slot. */
export interface RemoteEvent {
  /** Usually the client; an interceptor may put a run owner here instead. */
  sender: RemoteClient;
  senderFrame: { url: string } | null;
  returnValue?: unknown;
  preventDefault(): void;
}

const ASK_TIMEOUT_MS = 10 * 60 * 1000;

/** One browser tab. Looks enough like a WebContents for main's modules. */
export class RemoteClient extends EventEmitter {
  private destroyed = false;
  private nextAsk = 1;
  private readonly asks = new Map<number, { resolve(value: unknown): void; reject(error: Error): void; timer: NodeJS.Timeout }>();
  readonly mainFrame: { url: string };
  readonly window: RemoteWindow;
  /** 'tab' is a browser; 'engine' is the host's own document engine. */
  kind: 'tab' | 'engine' = 'tab';

  constructor(readonly id: number, private readonly socket: Socket, readonly url: string, private readonly ipc: WebIpcMain) {
    super();
    this.mainFrame = { url };
    this.window = new RemoteWindow(this);
  }

  /* ── WebContents surface used by main ───────────────── */
  send(channel: string, ...args: unknown[]): void {
    if (this.destroyed) return;
    this.push({ t: 'event', ch: channel, args });
  }
  isDestroyed(): boolean { return this.destroyed; }
  isLoading(): boolean { return false; }
  getURL(): string { return this.url; }
  /** Native edit actions are handled in the browser; nothing to do here. */
  undo(): void {} redo(): void {} cut(): void {} copy(): void {} paste(): void {} selectAll(): void {}
  capturePage(): Promise<never> { return Promise.reject(new Error('Window capture is not available on a remote client.')); }
  sendInputEvent(): void { throw new Error('Synthetic input is not available on a remote client.'); }
  executeJavaScript(): Promise<unknown> { return Promise.reject(new Error('executeJavaScript is not available on a remote client.')); }

  /** Server → browser question, e.g. a dialog the host cannot show. */
  ask<T = unknown>(channel: string, ...args: unknown[]): Promise<T> {
    if (this.destroyed) return Promise.reject(new Error('Powermove window is no longer available.'));
    const id = this.nextAsk++;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.asks.delete(id);
        reject(new Error('The browser did not answer in time.'));
      }, ASK_TIMEOUT_MS);
      timer.unref();
      this.asks.set(id, { resolve: (value) => resolve(value as T), reject, timer });
      this.push({ t: 'ask', id, ch: channel, args });
    });
  }

  /* ── socket plumbing ─────────────────────────────────── */
  receive(data: Uint8Array): void {
    if (this.destroyed) return;
    let message: unknown;
    try { message = decodeFrame(data); } catch (error) {
      console.warn(`[serve] client ${this.id}: bad frame (${error instanceof Error ? error.message : String(error)})`);
      return;
    }
    if (!isClientMessage(message)) return;
    void this.dispatch(message);
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    for (const [, pending] of this.asks) { clearTimeout(pending.timer); pending.reject(new Error('Powermove window closed.')); }
    this.asks.clear();
    this.emit('destroyed');
    this.window.emit('closed');
  }

  private push(message: ServerMessage): void {
    try { this.socket.send(encodeFrame(message)); } catch (error) {
      console.warn(`[serve] client ${this.id}: send failed (${error instanceof Error ? error.message : String(error)})`);
    }
  }

  private event(channel?: string, args: unknown[] = []): RemoteEvent {
    const stand = channel ? this.ipc.interceptors.get(channel)?.sender?.(this, args) : null;
    // A run owner has the WebContents members main's modules use; the type is nominal only.
    const sender = (stand ?? this) as unknown as RemoteClient;
    return { sender, senderFrame: this.mainFrame, preventDefault() {} };
  }

  private async dispatch(message: ClientMessage): Promise<void> {
    switch (message.t) {
      case 'invoke': {
        const handler = this.ipc.handlerFor(message.ch);
        if (!handler) { this.push({ t: 'result', id: message.id, ok: false, error: `No handler registered for '${message.ch}'` }); return; }
        const after = this.ipc.interceptors.get(message.ch)?.after;
        try {
          const value = await handler(this.event(message.ch, message.args), ...message.args);
          after?.(this, message.args, value, null);
          this.push({ t: 'result', id: message.id, ok: true, value: value ?? null });
        } catch (error) {
          after?.(this, message.args, undefined, error);
          this.push({ t: 'result', id: message.id, ok: false, error: error instanceof Error ? error.message : String(error) });
        }
        return;
      }
      case 'sync': {
        const event = this.event();
        try {
          this.ipc.emitOn(message.ch, event, ...message.args);
          this.push({ t: 'result', id: message.id, ok: true, value: event.returnValue ?? null });
        } catch (error) {
          this.push({ t: 'result', id: message.id, ok: false, error: error instanceof Error ? error.message : String(error) });
        }
        return;
      }
      case 'send':
        try { this.ipc.emitOn(message.ch, this.event(message.ch, message.args), ...message.args); } catch (error) {
          console.warn(`[serve] ${message.ch}: ${error instanceof Error ? error.message : String(error)}`);
        }
        return;
      case 'answer': {
        const pending = this.asks.get(message.id);
        if (!pending) return;
        clearTimeout(pending.timer);
        this.asks.delete(message.id);
        pending.resolve(message.value);
        return;
      }
    }
  }
}

/** The BrowserWindow a client "is". Window management is the browser tab's job. */
export class RemoteWindow extends EventEmitter {
  constructor(readonly webContents: RemoteClient) { super(); }
  isDestroyed(): boolean { return this.webContents.isDestroyed(); }
  isMinimized(): boolean { return false; }
  isFocused(): boolean { return true; }
  restore(): void {}
  show(): void {}
  focus(): void {}
  close(): void { this.webContents.destroy(); }
  destroy(): void { this.webContents.destroy(); }
  getBounds(): { x: number; y: number; width: number; height: number } { return { x: 0, y: 0, width: 1440, height: 900 }; }
}

/** IpcMain for the web host: one handler per invoke channel, listeners per one-way channel. */
export class WebIpcMain {
  private readonly handlers = new Map<string, InvokeHandler>();
  private readonly listeners = new Map<string, Set<Listener>>();
  private nextClientId = 1;
  private readonly clients = new Set<RemoteClient>();
  readonly interceptors = new Map<string, ChannelInterceptor>();

  intercept(channel: string, interceptor: ChannelInterceptor): void { this.interceptors.set(channel, interceptor); }

  handle(channel: string, handler: InvokeHandler): void {
    if (this.handlers.has(channel)) throw new Error(`Attempted to register a second handler for '${channel}'`);
    this.handlers.set(channel, handler);
  }
  removeHandler(channel: string): void { this.handlers.delete(channel); }
  on(channel: string, listener: Listener): this {
    let set = this.listeners.get(channel);
    if (!set) { set = new Set(); this.listeners.set(channel, set); }
    set.add(listener);
    return this;
  }
  once(channel: string, listener: Listener): this {
    const wrapped: Listener = (event, ...args) => { this.removeListener(channel, wrapped); listener(event, ...args); };
    return this.on(channel, wrapped);
  }
  removeListener(channel: string, listener: Listener): this {
    this.listeners.get(channel)?.delete(listener);
    return this;
  }
  off(channel: string, listener: Listener): this { return this.removeListener(channel, listener); }
  removeAllListeners(channel?: string): this {
    if (channel) this.listeners.delete(channel); else this.listeners.clear();
    return this;
  }

  handlerFor(channel: string): InvokeHandler | undefined { return this.handlers.get(channel); }
  hasListeners(channel: string): boolean { return (this.listeners.get(channel)?.size ?? 0) > 0; }
  emitOn(channel: string, event: RemoteEvent, ...args: unknown[]): void {
    const set = this.listeners.get(channel);
    if (!set) return;
    for (const listener of [...set]) listener(event, ...args);
  }

  /* ── connection registry ─────────────────────────────── */
  connect(socket: Socket, url: string): RemoteClient {
    const client = new RemoteClient(this.nextClientId++, socket, url, this);
    this.clients.add(client);
    client.once('destroyed', () => this.clients.delete(client));
    return client;
  }
  /** Live browser tabs (the engine is not a tab). */
  all(): RemoteClient[] { return [...this.clients].filter((client) => !client.isDestroyed() && client.kind === 'tab'); }
  /** The most recently connected live tab, for app-wide prompts. */
  current(): RemoteClient | null { return this.all().at(-1) ?? null; }
  engine(): RemoteClient | null { return [...this.clients].find((client) => !client.isDestroyed() && client.kind === 'engine') ?? null; }
  fromWebContents(contents: unknown): RemoteWindow | null {
    return contents instanceof RemoteClient && this.clients.has(contents) ? contents.window : null;
  }
}
