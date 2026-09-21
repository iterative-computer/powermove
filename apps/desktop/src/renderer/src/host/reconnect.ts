/*
 * A link to the host that survives the socket dropping. Wi-Fi blips, a laptop
 * lid, a phone changing networks and idle proxies all close the WebSocket;
 * without this every later call fails until the tab is reloaded, saves stop
 * landing and agent runs look dead even though the host kept going.
 *
 * The wrapper owns the listeners and answer handlers, so they carry over to
 * each new socket. Calls made while the link is down wait in order for the
 * next socket (up to a limit) instead of failing; calls in flight when the
 * socket dropped are rejected, since the host may or may not have run them.
 * Consumers that keep state on the host (project sessions, run attachment)
 * listen for `__reconnected` to re-establish it.
 */
import { Connection, type LinkSocket } from '../../../shared/link';

type Listener = (...args: unknown[]) => void;
type AskHandler = (...args: unknown[]) => Promise<unknown> | unknown;

export interface ReconnectingLinkOptions {
  /** Opens a socket; rejects when the host is unreachable right now. */
  open(): Promise<LinkSocket>;
  /** After each successful open, before queued calls go out. */
  handshake?(connection: Connection): Promise<void>;
  /** How long a call waits for the link to come back before failing. */
  waitMs?: number;
  /** Backoff bounds between attempts. */
  minDelayMs?: number;
  maxDelayMs?: number;
  /** Extra triggers to retry at once (browser `online`, tab becoming visible). */
  wake?(retry: () => void): () => void;
  log?(line: string): void;
}

export const DISCONNECTED_MESSAGE = 'The connection to the Powermove host is down. Waiting for it to come back…';

/** Errors from a call that ran into a dropped socket, so callers can retry after `__reconnected`. */
export function isDisconnectError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /connection to the Powermove host (closed|failed|is down)/.test(message);
}

interface Queued { run(connection: Connection): void; fail(error: Error): void; timer: ReturnType<typeof setTimeout> | null }

export class ReconnectingLink {
  private connection: Connection | null = null;
  private readonly listeners = new Map<string, Set<Listener>>();
  private readonly askHandlers = new Map<string, AskHandler>();
  private readonly queue: Queued[] = [];
  private closed = false;
  private attempt = 0;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private opening: Promise<void> | null = null;
  private unwake: (() => void) | null = null;
  /** Sockets seen so far; 1 after the first connect. */
  generation = 0;

  constructor(private readonly options: ReconnectingLinkOptions) {}

  /** Adopts an already-open connection (the boot handshake used it directly). */
  adopt(connection: Connection): void {
    this.attach(connection);
    this.unwake = this.options.wake?.(() => this.retryNow()) ?? null;
  }

  get isConnected(): boolean { return !!this.connection && !this.connection.isClosed; }
  get isClosed(): boolean { return this.closed; }

  invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T> {
    return this.withConnection((connection) => connection.invoke<T>(channel, ...args));
  }
  sync<T = unknown>(channel: string, ...args: unknown[]): Promise<T> {
    return this.withConnection((connection) => connection.sync<T>(channel, ...args));
  }
  /** One-way sends queue too: store writes must not vanish during a blip. */
  send(channel: string, ...args: unknown[]): void {
    if (this.closed) return;
    const connection = this.connection;
    if (connection && !connection.isClosed) { connection.send(channel, ...args); return; }
    this.enqueue({ run: (next) => next.send(channel, ...args), fail: () => undefined, timer: null });
  }
  on(channel: string, listener: Listener): () => void {
    let set = this.listeners.get(channel);
    if (!set) { set = new Set(); this.listeners.set(channel, set); }
    set.add(listener);
    if (this.connection) this.bind(this.connection, channel);
    return () => { set!.delete(listener); };
  }
  answer(channel: string, handler: AskHandler): void {
    this.askHandlers.set(channel, handler);
    this.connection?.answer(channel, handler);
  }

  /** Stops reconnecting for good (the tab is closing). */
  close(): void {
    this.closed = true;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.unwake?.();
    for (const item of this.queue.splice(0)) { if (item.timer) clearTimeout(item.timer); item.fail(new Error('The connection to the Powermove host closed.')); }
  }

  /* ── internals ────────────────────────────────────────── */

  private withConnection<T>(run: (connection: Connection) => Promise<T>): Promise<T> {
    if (this.closed) return Promise.reject(new Error('The connection to the Powermove host closed.'));
    const connection = this.connection;
    if (connection && !connection.isClosed) return run(connection);
    return new Promise<T>((resolve, reject) => {
      const item: Queued = {
        run: (next) => { run(next).then(resolve, reject); },
        fail: reject,
        timer: setTimeout(() => {
          const index = this.queue.indexOf(item);
          if (index >= 0) this.queue.splice(index, 1);
          reject(new Error(DISCONNECTED_MESSAGE));
        }, this.options.waitMs ?? 45_000)
      };
      this.enqueue(item);
    });
  }

  private enqueue(item: Queued): void {
    this.queue.push(item);
    this.retryNow();
  }

  /** Channels already forwarded from the current socket into our listener sets. */
  private bound = new Set<string>();

  private attach(connection: Connection): void {
    this.connection = connection;
    this.generation += 1;
    this.attempt = 0;
    this.bound = new Set();
    for (const channel of this.listeners.keys()) this.bind(connection, channel);
    for (const [channel, handler] of this.askHandlers) connection.answer(channel, handler);
    connection.on('__closed', () => { if (this.connection === connection) this.onDrop(); });
  }

  /** One forwarder per channel per socket; the set it reads is live, so later listeners are covered. */
  private bind(connection: Connection, channel: string): void {
    if (channel === '__closed' || channel === '__reconnected' || this.bound.has(channel)) return;
    this.bound.add(channel);
    connection.on(channel, (...args) => { for (const listener of [...(this.listeners.get(channel) ?? [])]) listener(...args); });
  }

  private onDrop(): void {
    this.connection = null;
    if (this.closed) return;
    this.options.log?.('[link] host connection dropped; reconnecting');
    this.emit('__closed');
    this.scheduleRetry(0);
  }

  private scheduleRetry(delay: number): void {
    if (this.closed || this.retryTimer || this.opening) return;
    this.retryTimer = setTimeout(() => { this.retryTimer = null; void this.tryOpen(); }, delay);
  }

  private retryNow(): void {
    if (this.closed || this.connection || this.opening) return;
    if (this.retryTimer) { clearTimeout(this.retryTimer); this.retryTimer = null; }
    void this.tryOpen();
  }

  private async tryOpen(): Promise<void> {
    if (this.closed || this.connection || this.opening) return;
    this.opening = (async () => {
      try {
        const socket = await this.options.open();
        const connection = new Connection(socket);
        await this.options.handshake?.(connection);
        if (this.closed) { socket.close(); return; }
        this.attach(connection);
        this.options.log?.('[link] reconnected to the host');
        this.emit('__reconnected');
        for (const item of this.queue.splice(0)) { if (item.timer) clearTimeout(item.timer); item.run(connection); }
      } catch {
        const min = this.options.minDelayMs ?? 500;
        const max = this.options.maxDelayMs ?? 10_000;
        const delay = Math.min(max, min * 2 ** Math.min(this.attempt++, 10));
        this.opening = null;
        this.scheduleRetry(delay);
        return;
      }
      this.opening = null;
    })();
    await this.opening;
  }

  private emit(channel: string): void {
    for (const listener of [...(this.listeners.get(channel) ?? [])]) {
      try { listener(); } catch (error) { console.error(`[link] ${channel}`, error); }
    }
  }
}
