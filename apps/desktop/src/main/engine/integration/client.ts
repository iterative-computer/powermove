/*
 * A protocol client that plays a renderer window in the main-process
 * integration test: it opens a connection on a port main delivered,
 * negotiates as a renderer, and sends requests.
 */
import { PROTOCOL_BUILD, type EngineProcess, type Envelope, type Request, type Response } from '../../../shared/engine-protocol';
import { MAX_MESSAGE_BYTES } from '../msgpack';
import type { PortLike } from '../ports';
import { completeWelcome, decodeEnvelope, encodeEnvelope, engineRole, envelope, localRanges, Outstanding, requestMessage } from '../protocol';

export class TestClient {
  private outstanding = new Outstanding();
  private welcome: ((ok: boolean) => void) | null = null;
  readonly events: Envelope[] = [];
  closed = false;

  constructor(readonly process: EngineProcess, readonly generation: number, private readonly port: PortLike) {
    port.on('message', (event) => this.receive(event.data as Uint8Array));
    port.on('close', () => {
      this.closed = true;
      this.outstanding.failAll(new Error(`the ${process} engine port closed`));
    });
    port.start();
  }

  open(): Promise<void> {
    const done = new Promise<void>((resolve, reject) => {
      this.welcome = (ok) => (ok ? resolve() : reject(new Error('handshake refused')));
    });
    this.port.postMessage(encodeEnvelope(envelope({ kind: 'hello', role: 'renderer', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges(), maxMessageBytes: MAX_MESSAGE_BYTES })));
    return done;
  }

  request(body: Request, session: number | null = null): Promise<Response> {
    const { id, answer } = this.outstanding.open(body);
    this.port.postMessage(encodeEnvelope(envelope(requestMessage(id, body), session)));
    return answer;
  }

  close(): void {
    this.port.close();
  }

  private receive(bytes: Uint8Array): void {
    const message = decodeEnvelope(bytes);
    const body = message.message;
    if (this.welcome) {
      const settle = this.welcome;
      this.welcome = null;
      if (body.kind !== 'welcome') return settle(false);
      try {
        completeWelcome(body, engineRole(this.process));
        settle(true);
      } catch {
        settle(false);
      }
      return;
    }
    if (body.kind === 'response' || body.kind === 'error') this.outstanding.settle(body);
    else this.events.push(message);
  }
}
