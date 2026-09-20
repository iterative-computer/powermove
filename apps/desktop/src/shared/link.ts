/*
 * One WebSocket to the host, speaking the wire protocol: invoke/sync requests
 * with numbered replies, one-way sends, host → client events, and host
 * questions ("ask") the client answers. DOM-free so the browser bridge and
 * the Node document engine share it.
 */
import { decodeFrame, encodeFrame, type ServerMessage } from './wire';

/** The subset of WebSocket both browsers and `ws` provide. */
export interface LinkSocket {
  binaryType: string;
  send(data: Uint8Array): void;
  close(): void;
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void;
  addEventListener(type: 'close' | 'error', listener: () => void): void;
}

type Listener = (...args: unknown[]) => void;

export class Connection {
  private nextId = 1;
  private readonly pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>();
  private readonly listeners = new Map<string, Set<Listener>>();
  private readonly askHandlers = new Map<string, (...args: unknown[]) => Promise<unknown> | unknown>();
  private closed = false;

  constructor(private readonly socket: LinkSocket) {
    socket.binaryType = 'arraybuffer';
    socket.addEventListener('message', (event) => this.receive(toBytes(event.data)));
    socket.addEventListener('close', () => this.fail(new Error('The connection to the Powermove host closed.')));
    socket.addEventListener('error', () => this.fail(new Error('The connection to the Powermove host failed.')));
  }

  invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T> { return this.request<T>('invoke', channel, args); }
  /** ipcRenderer.sendSync stand-in: async on the wire, answered from the `on` listener. */
  sync<T = unknown>(channel: string, ...args: unknown[]): Promise<T> { return this.request<T>('sync', channel, args); }
  send(channel: string, ...args: unknown[]): void { this.socket.send(encodeFrame({ t: 'send', ch: channel, args })); }
  on(channel: string, listener: Listener): () => void {
    let set = this.listeners.get(channel);
    if (!set) { set = new Set(); this.listeners.set(channel, set); }
    set.add(listener);
    return () => { set!.delete(listener); };
  }
  /** Register the browser's answer to a server question (dialogs). */
  answer(channel: string, handler: (...args: unknown[]) => Promise<unknown> | unknown): void { this.askHandlers.set(channel, handler); }
  get isClosed(): boolean { return this.closed; }

  private request<T>(kind: 'invoke' | 'sync', channel: string, args: unknown[]): Promise<T> {
    if (this.closed) return Promise.reject(new Error('The connection to the Powermove host closed.'));
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: (value) => resolve(value as T), reject });
      try { this.socket.send(encodeFrame({ t: kind, id, ch: channel, args })); } catch (error) {
        this.pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  private receive(data: Uint8Array): void {
    let message: ServerMessage;
    try { message = decodeFrame(data) as ServerMessage; } catch (error) { console.warn('[web-bridge] bad frame', error); return; }
    switch (message.t) {
      case 'result': {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.ok) pending.resolve(message.value); else pending.reject(new Error(message.error));
        return;
      }
      case 'event':
        for (const listener of [...(this.listeners.get(message.ch) ?? [])]) {
          try { listener(...message.args); } catch (error) { console.error(`[web-bridge] ${message.ch}`, error); }
        }
        return;
      case 'ask': {
        const handler = this.askHandlers.get(message.ch);
        void Promise.resolve(handler ? handler(...message.args) : null).catch(() => null).then((value) => {
          this.socket.send(encodeFrame({ t: 'answer', id: message.id, value: value ?? null }));
        });
        return;
      }
    }
  }

  private fail(error: Error): void {
    if (this.closed) return;
    this.closed = true;
    for (const [, pending] of this.pending) pending.reject(error);
    this.pending.clear();
    for (const listener of [...(this.listeners.get('__closed') ?? [])]) listener();
  }
}


function toBytes(data: unknown): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (Array.isArray(data)) {
    const parts = data as Uint8Array[];
    const out = new Uint8Array(parts.reduce((total, part) => total + part.byteLength, 0));
    let offset = 0;
    for (const part of parts) { out.set(part, offset); offset += part.byteLength; }
    return out;
  }
  throw new Error('link: expected binary frame');
}
