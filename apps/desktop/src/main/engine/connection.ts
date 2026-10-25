/*
 * Main's connection to one engine launch over its host port (contract 9).
 *
 * Main opens the connection: it sends `hello`, checks the engine's
 * `welcome` (role, wire version, build, every contract), and from then on
 * correlates its own requests (`grantLease`, sessions main opens) and
 * serves the engine's requests to the host (handles, outputs, leases,
 * hardware sessions). Anything malformed from the engine is answered with
 * an error and logged; main never trusts the engine's word.
 */
import type { EngineError, EngineProcess, Envelope, Event, Message, Request, Response } from '../../shared/engine-protocol';
import type { PortLike } from './ports';
import {
  completeWelcome,
  decodeEnvelope,
  encodeEnvelope,
  engineError,
  ERROR_CODES,
  engineRole,
  envelope,
  errorMessage,
  eventMessage,
  hostHello,
  type Negotiated,
  Outstanding,
  ProtocolError,
  requestMessage,
  responseMessage
} from './protocol';

export type Log = (level: 'info' | 'warn' | 'error', message: string) => void;

/** What the connection hands to host services. */
export interface HostHandler {
  /** An engine → host request. Resolve with the response, or throw an
   *  error that becomes the error answer (`ProtocolError` and friends carry
   *  a code; anything else is `internal`). */
  request(body: Request, session: number | null): Promise<Response>;
  /** An engine → host event. */
  event(body: Event, session: number | null): void;
}

export const HANDSHAKE_TIMEOUT_MS = 10_000;

function errorFor(error: unknown, id: number): EngineError {
  const code = (error as { code?: unknown })?.code;
  const known = typeof code === 'string' && ERROR_CODES.has(code as EngineError['code']) ? code : 'internal';
  const message = error instanceof Error && error.message ? error.message : 'The host could not serve the request.';
  return engineError(known as EngineError['code'], { kind: 'request', id }, message.slice(0, 4096));
}

export class HostConnection {
  private outstanding = new Outstanding();
  private negotiated: Negotiated | null = null;
  private closed = false;
  private handshake: { resolve: (n: Negotiated) => void; reject: (e: Error) => void } | null = null;
  private closeListeners: Array<(reason: Error) => void> = [];

  constructor(
    readonly process: EngineProcess,
    readonly portId: number,
    private readonly port: PortLike,
    private readonly handler: HostHandler,
    private readonly log: Log
  ) {
    port.on('message', (event) => this.receive(event.data));
    port.on('close', () => this.shutdown(new ProtocolError('internal', `the ${process} engine's host port closed`)));
    port.start();
  }

  /** Sends hello and waits for a valid welcome. */
  open(timeoutMs = HANDSHAKE_TIMEOUT_MS): Promise<Negotiated> {
    if (this.handshake || this.negotiated) throw new Error('the handshake already started');
    const result = new Promise<Negotiated>((resolve, reject) => {
      this.handshake = { resolve, reject };
    });
    const timer = setTimeout(() => this.shutdown(new ProtocolError('timeout', `the ${this.process} engine did not answer hello within ${timeoutMs} ms`)), timeoutMs);
    result.then(() => clearTimeout(timer), () => clearTimeout(timer));
    this.send(envelope({ kind: 'hello', ...hostHello() }));
    return result;
  }

  get isOpen(): boolean {
    return this.negotiated !== null && !this.closed;
  }

  /** Sends a request and resolves with the engine's answer. */
  request(body: Request, session: number | null = null, revision: number | null = null, signal?: AbortSignal): Promise<Response> {
    if (signal?.aborted) return Promise.reject(new ProtocolError('cancelled', 'The request was cancelled.'));
    if (!this.isOpen) return Promise.reject(new ProtocolError('internal', `the ${this.process} engine is not connected`));
    const { id, answer } = this.outstanding.open(body);
    this.send(envelope(requestMessage(id, body), session, revision));
    const cancel = () => this.send(envelope({ kind: 'cancel', id }, session));
    signal?.addEventListener('abort', cancel, { once: true });
    return answer.finally(() => signal?.removeEventListener('abort', cancel));
  }

  event(body: Event, session: number | null = null): void {
    if (!this.isOpen) return;
    this.send(envelope(eventMessage(body), session));
  }

  close(): void {
    this.shutdown(new ProtocolError('internal', `the ${this.process} engine connection closed`));
  }

  /** Called once when the connection ends, for any reason. */
  onClose(listener: (reason: Error) => void): void {
    if (this.closed) listener(new ProtocolError('internal', 'already closed'));
    else this.closeListeners.push(listener);
  }

  private send(value: Envelope): void {
    if (this.closed) return;
    let bytes: Uint8Array;
    try {
      bytes = encodeEnvelope(value);
    } catch (error) {
      this.log('error', `could not encode a message for the ${this.process} engine: ${String(error)}`);
      return;
    }
    if (this.negotiated && bytes.byteLength > this.negotiated.maxMessageBytes) {
      this.log('error', `dropped a ${bytes.byteLength}-byte message: the ${this.process} engine accepts at most ${this.negotiated.maxMessageBytes}`);
      return;
    }
    this.port.postMessage(bytes);
  }

  private shutdown(reason: Error): void {
    if (this.closed) return;
    this.closed = true;
    this.handshake?.reject(reason);
    this.handshake = null;
    this.outstanding.failAll(reason);
    try { this.port.close(); } catch { /* already closed */ }
    for (const listener of this.closeListeners.splice(0)) listener(reason);
  }

  private receive(data: unknown): void {
    if (this.closed) return;
    const bytes = data instanceof Uint8Array ? data : ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : data instanceof ArrayBuffer ? new Uint8Array(data) : null;
    if (!bytes) {
      this.log('warn', `non-binary message from the ${this.process} engine; ignored`);
      return;
    }
    let message: Envelope;
    try {
      message = decodeEnvelope(bytes);
    } catch (error) {
      this.log('error', `undecodable message from the ${this.process} engine: ${error instanceof Error ? error.message : String(error)}`);
      if (!this.negotiated) this.shutdown(error instanceof Error ? error : new Error(String(error)));
      return;
    }
    if (!this.negotiated) {
      this.onHandshake(message.message);
      return;
    }
    this.dispatch(message);
  }

  private onHandshake(message: Message): void {
    if (message.kind === 'welcome') {
      try {
        this.negotiated = completeWelcome(message, engineRole(this.process));
      } catch (error) {
        this.shutdown(error instanceof Error ? error : new Error(String(error)));
        return;
      }
      this.handshake?.resolve(this.negotiated);
      this.handshake = null;
      return;
    }
    const reason = message.kind === 'error' ? `the ${this.process} engine refused the handshake: ${message.error.message}` : `expected welcome from the ${this.process} engine, got ${message.kind}`;
    this.shutdown(new ProtocolError(message.kind === 'error' ? message.error.code : 'unsupportedVersion', reason));
  }

  private dispatch(message: Envelope): void {
    const body = message.message;
    switch (body.kind) {
      case 'response':
      case 'error':
        if (!this.outstanding.settle(body)) {
          this.log('warn', `the ${this.process} engine answered request ${body.id}, which is not outstanding${body.kind === 'error' ? `: ${body.error.message}` : ''}`);
        }
        return;
      case 'request': {
        const { id } = body;
        this.handler.request(body.body, message.session).then(
          (response) => this.send(envelope(responseMessage(id, response), message.session)),
          (error: unknown) => {
            const answer = errorFor(error, id);
            if (answer.code === 'internal') this.log('error', `host request ${body.body.type} from the ${this.process} engine failed: ${answer.message}`);
            this.send(envelope(errorMessage(id, answer)));
          }
        );
        return;
      }
      case 'event':
        try {
          this.handler.event(body.body, message.session);
        } catch (error) {
          this.log('warn', `event ${body.body.type} from the ${this.process} engine was refused: ${error instanceof Error ? error.message : String(error)}`);
        }
        return;
      case 'cancel':
        // Host requests are answered as soon as their I/O finishes; there
        // is nothing to stop midway.
        return;
      case 'hello':
      case 'welcome':
        this.log('error', `the ${this.process} engine repeated the handshake; closing`);
        this.close();
        return;
    }
  }
}
