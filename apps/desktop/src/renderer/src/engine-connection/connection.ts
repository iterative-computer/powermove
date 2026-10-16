/*
 * One connection from this window to the interactive engine (plan section
 * 19; contract 9): the `MessagePort` main posted for the current engine
 * launch, its handshake, requests with their answers, and session-bound
 * events routed to whoever holds the session.
 *
 * A new engine launch (a restart after a crash) arrives as a new port with
 * a new generation: the old connection closes, every session on it ends,
 * and the kernel's engine connection opens new sessions and resyncs them.
 */
import { PROTOCOL_BUILD, type Envelope, type Event, type Request, type Response } from '../../../shared/engine-protocol';
import { MAX_MESSAGE_BYTES } from '../../../main/engine/msgpack';
import { completeWelcome, decodeEnvelope, encodeEnvelope, engineRole, envelope, eventMessage, localRanges, Outstanding, requestMessage } from '../../../main/engine/protocol';

/** The part of a `MessagePort` the connection uses (a fake in tests). */
export interface PortLike {
  postMessage(message: unknown): void;
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void;
  addEventListener(type: 'close', listener: () => void): void;
  start(): void;
  close(): void;
}

/** A session's messages: each decoded envelope with its arrival time. */
export type SessionListener = (message: Envelope, arrivedMs: number) => void;

export type Log = (level: 'info' | 'warn' | 'error', message: string) => void;

export class EngineConnection {
  private readonly outstanding = new Outstanding();
  private welcome: ((ok: boolean, reason?: string) => void) | null = null;
  private readonly sessions = new Map<number, SessionListener>();
  private readonly closers = new Set<(reason: string) => void>();
  ready = false;
  closed = false;
  /** Messages the connection could not decode (logged once each kind). */
  readonly errors: string[] = [];

  constructor(
    readonly generation: number,
    private readonly port: PortLike,
    private readonly log: Log,
    private readonly now: () => number = () => performance.now()
  ) {
    port.addEventListener('message', (event) => this.receive(event.data));
    port.addEventListener('close', () => this.close('the engine port closed'));
    port.start();
  }

  /** The handshake (`hello` as a renderer, the engine's `welcome`). */
  open(): Promise<void> {
    const answered = new Promise<void>((resolve, reject) => {
      this.welcome = (ok, reason) => (ok ? resolve() : reject(new Error(reason ?? 'the interactive engine refused the handshake')));
    });
    this.post(envelope({ kind: 'hello', role: 'renderer', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges(), maxMessageBytes: MAX_MESSAGE_BYTES }));
    return answered.then(() => {
      this.ready = true;
    });
  }

  request(body: Request, session: number | null = null, revision: number | null = null): Promise<Response> {
    if (this.closed) return Promise.reject(new Error('the engine connection closed'));
    const { id, answer } = this.outstanding.open(body);
    this.post(envelope(requestMessage(id, body), session, revision));
    return answer;
  }

  event(body: Event, session: number, revision: number | null = null): void {
    if (this.closed) return;
    this.post(envelope(eventMessage(body), session, revision));
  }

  /** Routes the session's events (and nothing else) to `listener`. */
  listen(session: number, listener: SessionListener): () => void {
    this.sessions.set(session, listener);
    return () => {
      if (this.sessions.get(session) === listener) this.sessions.delete(session);
    };
  }

  /** Called once when the connection closes. */
  onClose(listener: (reason: string) => void): () => void {
    if (this.closed) {
      listener('the engine connection closed');
      return () => {};
    }
    this.closers.add(listener);
    return () => this.closers.delete(listener);
  }

  close(reason: string): void {
    if (this.closed) return;
    this.closed = true;
    this.ready = false;
    try {
      this.port.close();
    } catch {
      /* already closed */
    }
    this.outstanding.failAll(new Error(reason));
    this.welcome?.(false, reason);
    this.welcome = null;
    this.sessions.clear();
    const closers = [...this.closers];
    this.closers.clear();
    for (const listener of closers) listener(reason);
  }

  private post(message: Envelope): void {
    try {
      this.port.postMessage(encodeEnvelope(message));
    } catch (error) {
      this.log('warn', `engine message not sent: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  private receive(data: unknown): void {
    if (this.closed) return;
    const arrived = this.now();
    const bytes = data instanceof Uint8Array ? data : data instanceof ArrayBuffer ? new Uint8Array(data) : null;
    if (!bytes) return;
    let message: Envelope;
    try {
      message = decodeEnvelope(bytes);
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      if (!this.errors.includes(text)) {
        this.errors.push(text);
        this.log('warn', `engine message not decoded: ${text}`);
      }
      return;
    }
    const body = message.message;
    if (this.welcome) {
      const settle = this.welcome;
      this.welcome = null;
      if (body.kind !== 'welcome') {
        settle(false, body.kind === 'error' ? body.error.message : `expected welcome, got ${body.kind}`);
        return;
      }
      try {
        completeWelcome(body, engineRole('interactive'));
        settle(true);
      } catch (error) {
        settle(false, error instanceof Error ? error.message : String(error));
      }
      return;
    }
    if (body.kind === 'response' || body.kind === 'error') {
      // A request-scoped answer; a connection-scope error (id 0 with no
      // request) closes the connection on the engine's side as well.
      if (!this.outstanding.settle(body) && body.kind === 'error' && body.error.scope.kind === 'connection') {
        this.close(`the engine closed the connection: ${body.error.message}`);
      }
      return;
    }
    if (body.kind !== 'event' || message.session === null) return;
    this.sessions.get(message.session)?.(message, arrived);
  }
}
