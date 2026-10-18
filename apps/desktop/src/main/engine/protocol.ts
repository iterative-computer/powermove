/*
 * The engine protocol as main speaks it (contract 9): envelopes, the
 * host's side of version negotiation, request correlation, and the guards
 * main runs on everything an engine sends. Types are generated from
 * pm-protocol (src/shared/engine-protocol); the rules mirror
 * pm_protocol::version and pm_protocol::envelope, and the fixtures test
 * (protocol.test.ts) holds the two together.
 */
import {
  PROTOCOL_BUILD,
  type ContractId,
  type ContractRange,
  type EngineError,
  type EngineProcess,
  type Envelope,
  type ErrorCode,
  type ErrorScope,
  type Event,
  type Hello,
  type Message,
  type PeerRole,
  type Request,
  type Response,
  type Welcome
} from '../../shared/engine-protocol';
import { decode, encode, MAX_MESSAGE_BYTES } from './msgpack';

export const WIRE_VERSION = 1;
export const MIN_MESSAGE_LIMIT = 64 * 1024;

export const CONTRACTS: readonly ContractId[] = [
  'evaluatedScene', 'frame', 'gpuInterop', 'color', 'patch', 'clock', 'errors', 'lease',
  'protocol', 'definitions', 'blender', 'priority', 'slicing', 'hostServices', 'documentHost'
];

/** Every contract at its current version (all drafts, 0), all required. */
export function localRanges(): ContractRange[] {
  return CONTRACTS.map((contract) => ({ contract, min: 0, max: 0, required: true }));
}

export function engineRole(process: EngineProcess): PeerRole {
  return process === 'interactive' ? 'interactiveEngine' : 'jobEngine';
}

export function hostHello(build = PROTOCOL_BUILD): Hello {
  return { role: 'host', wire: WIRE_VERSION, build, contracts: localRanges(), maxMessageBytes: MAX_MESSAGE_BYTES };
}

export const ERROR_CODES: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
  'invalidMessage', 'unsupportedVersion', 'unknownSession', 'staleRevision', 'capabilityRejected',
  'definitionMissing', 'shaderCompile', 'watchdogTimeout', 'decodeFailed', 'mediaOffline',
  'budgetExceeded', 'deviceLost', 'blenderMissing', 'blenderFailed', 'notHeadlessRenderable',
  'exportFailed', 'cancelled', 'timeout', 'unsupported', 'crashAttributed', 'internal'
]);

export class ProtocolError extends Error {
  override name = 'ProtocolError';
  constructor(readonly code: ErrorCode, message: string) {
    super(message);
  }
}

/** What a connection agreed on. */
export type Negotiated = { versions: Map<ContractId, number>; maxMessageBytes: number; peer: PeerRole };

/** The opening side's check of a welcome (pm_protocol::version::complete). */
export function completeWelcome(welcome: Welcome, expected: PeerRole, offered: ContractRange[] = localRanges(), build = PROTOCOL_BUILD): Negotiated {
  if (welcome.wire !== WIRE_VERSION) throw new ProtocolError('unsupportedVersion', `wire version mismatch: local ${WIRE_VERSION}, remote ${welcome.wire}`);
  if (welcome.role !== expected) throw new ProtocolError('capabilityRejected', `expected a ${expected}, the peer is a ${welcome.role}`);
  if (!(welcome.maxMessageBytes >= MIN_MESSAGE_LIMIT)) throw new ProtocolError('unsupportedVersion', `the peer accepts at most ${welcome.maxMessageBytes} bytes; the floor is 64 KiB`);
  const versions = new Map<ContractId, number>();
  for (const agreed of welcome.contracts) {
    const range = offered.find((r) => r.contract === agreed.contract);
    if (!range) throw new ProtocolError('unsupportedVersion', `the peer agreed to ${agreed.contract}, which we never offered`);
    if (agreed.version < range.min || agreed.version > range.max) throw new ProtocolError('unsupportedVersion', `the peer agreed to ${agreed.contract} version ${agreed.version}, outside our range`);
    if (agreed.version === 0 && welcome.build !== build) throw new ProtocolError('unsupportedVersion', `draft contract ${agreed.contract} needs equal builds (local ${build}, remote ${welcome.build})`);
    if (versions.has(agreed.contract)) throw new ProtocolError('unsupportedVersion', `contract ${agreed.contract} is listed twice`);
    versions.set(agreed.contract, agreed.version);
  }
  for (const range of offered) {
    if (range.required && !versions.has(range.contract)) throw new ProtocolError('unsupportedVersion', `contract ${range.contract} is required but the peer does not speak it`);
  }
  return { versions, maxMessageBytes: Math.min(welcome.maxMessageBytes, MAX_MESSAGE_BYTES), peer: welcome.role };
}

/* ── guards ──────────────────────────────────────────────────────────── */

export const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value) && !(value instanceof Uint8Array);
export const isSafeUint = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
export const isHandle = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{32}$/.test(value);
export const isId = (value: unknown): value is string =>
  typeof value === 'string' && value.length > 0 && Buffer.byteLength(value) <= 256;

const MESSAGE_KINDS = new Set(['hello', 'welcome', 'request', 'response', 'error', 'event', 'cancel']);

/** Decodes and shape-checks an envelope. Field-level checks happen where
 *  each message is handled; anything malformed throws `ProtocolError`. */
export function decodeEnvelope(bytes: Uint8Array): Envelope {
  let value: unknown;
  try {
    value = decode(bytes);
  } catch (error) {
    throw new ProtocolError('invalidMessage', error instanceof Error ? error.message : String(error));
  }
  if (!isRecord(value)) throw new ProtocolError('invalidMessage', 'an envelope is a map');
  if (value['wire'] !== WIRE_VERSION) throw new ProtocolError('unsupportedVersion', `wire version ${String(value['wire'])}`);
  const session = value['session'] ?? null;
  const revision = value['revision'] ?? null;
  if (session !== null && !isSafeUint(session)) throw new ProtocolError('invalidMessage', 'bad session');
  if (revision !== null && !isSafeUint(revision)) throw new ProtocolError('invalidMessage', 'bad revision');
  const message = value['message'];
  if (!isRecord(message) || typeof message['kind'] !== 'string' || !MESSAGE_KINDS.has(message['kind'])) {
    throw new ProtocolError('invalidMessage', 'unknown message kind');
  }
  const kind = message['kind'];
  if ((kind === 'request' || kind === 'response' || kind === 'error' || kind === 'cancel') && !isSafeUint(message['id'])) {
    throw new ProtocolError('invalidMessage', 'bad request id');
  }
  if ((kind === 'request' || kind === 'response' || kind === 'event') && (!isRecord(message['body']) || typeof message['body']['type'] !== 'string')) {
    throw new ProtocolError('invalidMessage', 'a body is tagged by type');
  }
  if (kind === 'error') {
    const error = message['error'];
    if (!isRecord(error) || typeof error['code'] !== 'string' || typeof error['message'] !== 'string' || !isRecord(error['scope'])) {
      throw new ProtocolError('invalidMessage', 'bad error');
    }
  }
  return { wire: WIRE_VERSION, session: session as number | null, revision: revision as number | null, message: message as unknown as Message };
}

export function encodeEnvelope(envelope: Envelope): Uint8Array {
  return encode(envelope);
}

export function envelope(message: Message, session: number | null = null, revision: number | null = null): Envelope {
  return { wire: WIRE_VERSION, session, revision, message };
}

export const requestMessage = (id: number, body: Request): Message => ({ kind: 'request', id, body });
export const responseMessage = (id: number, body: Response): Message => ({ kind: 'response', id, body });
export const eventMessage = (body: Event): Message => ({ kind: 'event', body });
export const errorMessage = (id: number, error: EngineError): Message => ({ kind: 'error', id, error });

export function engineError(code: ErrorCode, scope: ErrorScope, message: string): EngineError {
  return { code, scope, message, retry: { kind: 'never' }, detail: null };
}

/** The response type that answers each request type (pm_protocol `Request::expects`). */
export const RESPONSE_FOR: Readonly<Record<Request['type'], Response['type'] | readonly Response['type'][]>> = {
  openSession: 'sessionOpened',
  closeSession: 'ack',
  resync: 'patchesApplied',
  checkConsistency: 'consistency',
  renderFrame: ['frame', 'image'],
  evaluate: 'scene',
  agentFrames: 'agentFrames',
  submitJob: 'jobAccepted',
  cancelJobs: 'jobsCancelled',
  pushDefinitions: 'definitionsApplied',
  grantLease: 'leaseUsage',
  requestLease: 'lease',
  mintHandle: 'handle',
  resolveHandle: 'handleResolved',
  outputDestination: 'outputDestination',
  commitOutput: 'outputCommitted',
  hardwareSessions: 'hardwareSessions',
  blenderRender: 'ack',
  blenderStatus: 'blenderStatus',
  blenderModel: 'ack',
  openProject: 'projectOpened',
  closeProject: 'ack',
  persist: 'persisted',
  registerExtension: 'extensionRegistered',
  toolCall: 'toolResult',
  probeMedia: 'mediaFacts',
  snapshot: 'snapshot',
  compatFrame: 'compatFrame'
};

type Pending = { expects: readonly Response['type'][]; resolve: (response: Response) => void; reject: (error: Error) => void };

/** Error from the other side, as a JavaScript error. */
export class RemoteError extends Error {
  override name = 'RemoteError';
  constructor(readonly error: EngineError) {
    super(error.message);
  }
}

/** One direction's outstanding requests (pm_protocol `Outstanding`). */
export class Outstanding {
  private next = 0;
  private pending = new Map<number, Pending>();

  /** Allocates an id and a promise for the answer. */
  open(request: Request): { id: number; answer: Promise<Response> } {
    const id = this.next++;
    if (!Number.isSafeInteger(this.next)) throw new ProtocolError('internal', 'request ids are exhausted on this connection');
    const expected = RESPONSE_FOR[request.type];
    const expects = typeof expected === 'string' ? [expected] : expected;
    const answer = new Promise<Response>((resolve, reject) => this.pending.set(id, { expects, resolve, reject }));
    // Callers that drop the promise must not crash main on rejection.
    answer.catch(() => {});
    return { id, answer };
  }

  /** Settles the request a response or error answers. Returns false when
   *  nothing was waiting for it (a protocol violation by the peer). */
  settle(message: Message): boolean {
    if (message.kind !== 'response' && message.kind !== 'error') return false;
    const pending = this.pending.get(message.id);
    if (!pending) return false;
    this.pending.delete(message.id);
    if (message.kind === 'error') {
      pending.reject(new RemoteError(message.error));
      return true;
    }
    if (!pending.expects.includes(message.body.type)) {
      pending.reject(new ProtocolError('invalidMessage', `expected ${pending.expects.join(' or ')}, got ${message.body.type}`));
      return true;
    }
    pending.resolve(message.body);
    return true;
  }

  /** Fails everything outstanding (the connection closed). */
  failAll(reason: Error): void {
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const p of pending) p.reject(reason);
  }

  get size(): number {
    return this.pending.size;
  }
}
