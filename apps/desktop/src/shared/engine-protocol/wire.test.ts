// Hand-written (not generated): checks that the golden MessagePack fixtures
// written by engine/crates/pm-protocol decode into objects that match the
// generated TypeScript types. Field names are checked by the compiler through
// typed access below; values and shapes are checked at runtime.
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import type { Envelope, EvaluatedScene, FrameDescriptor, FrameRequest, Hello } from './index';

const FIXTURES = path.resolve(__dirname, '../../../../../engine/crates/pm-protocol/tests/fixtures/wire');

/** A strict MessagePack decoder for the subset the protocol uses (no ext types). */
function decodeMessagePack(bytes: Uint8Array): unknown {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const text = new TextDecoder('utf-8', { fatal: true });
  let at = 0;
  const take = (n: number): number => {
    if (at + n > bytes.length) throw new Error(`truncated at ${at}`);
    const start = at;
    at += n;
    return start;
  };
  const safe = (value: bigint): number => {
    if (value > BigInt(Number.MAX_SAFE_INTEGER) || value < BigInt(-Number.MAX_SAFE_INTEGER)) throw new Error(`unsafe integer ${value}`);
    return Number(value);
  };
  const str = (n: number) => text.decode(bytes.subarray(take(n), at));
  const bin = (n: number) => bytes.slice(take(n), at);
  const array = (n: number): unknown[] => Array.from({ length: n }, () => value());
  const map = (n: number): Record<string, unknown> => {
    const out: Record<string, unknown> = {};
    for (let i = 0; i < n; i++) {
      const key = value();
      if (typeof key !== 'string') throw new Error('map keys are strings');
      if (key in out) throw new Error(`duplicate key ${key}`);
      out[key] = value();
    }
    return out;
  };
  function value(): unknown {
    const byte = view.getUint8(take(1));
    if (byte <= 0x7f) return byte;
    if (byte >= 0xe0) return byte - 0x100;
    if ((byte & 0xe0) === 0xa0) return str(byte & 0x1f);
    if ((byte & 0xf0) === 0x90) return array(byte & 0x0f);
    if ((byte & 0xf0) === 0x80) return map(byte & 0x0f);
    switch (byte) {
      case 0xc0: return null;
      case 0xc2: return false;
      case 0xc3: return true;
      case 0xc4: return bin(view.getUint8(take(1)));
      case 0xc5: return bin(view.getUint16(take(2)));
      case 0xc6: return bin(view.getUint32(take(4)));
      case 0xca: return view.getFloat32(take(4));
      case 0xcb: return view.getFloat64(take(8));
      case 0xcc: return view.getUint8(take(1));
      case 0xcd: return view.getUint16(take(2));
      case 0xce: return view.getUint32(take(4));
      case 0xcf: return safe(view.getBigUint64(take(8)));
      case 0xd0: return view.getInt8(take(1));
      case 0xd1: return view.getInt16(take(2));
      case 0xd2: return view.getInt32(take(4));
      case 0xd3: return safe(view.getBigInt64(take(8)));
      case 0xd9: return str(view.getUint8(take(1)));
      case 0xda: return str(view.getUint16(take(2)));
      case 0xdb: return str(view.getUint32(take(4)));
      case 0xdc: return array(view.getUint16(take(2)));
      case 0xdd: return array(view.getUint32(take(4)));
      case 0xde: return map(view.getUint16(take(2)));
      case 0xdf: return map(view.getUint32(take(4)));
      default: throw new Error(`unsupported MessagePack byte 0x${byte.toString(16)} at ${at - 1}`);
    }
  }
  const result = value();
  if (at !== bytes.length) throw new Error(`${bytes.length - at} trailing bytes`);
  return result;
}

function load(name: string): Envelope {
  return decodeMessagePack(readFileSync(path.join(FIXTURES, `${name}.msgpack`))) as Envelope;
}

const files = readdirSync(FIXTURES).filter((f) => f.endsWith('.msgpack')).sort();

describe('engine protocol wire fixtures', () => {
  it('has a fixture for every message family', () => {
    expect(files.length).toBeGreaterThan(60);
    for (const prefix of ['hello', 'welcome', 'cancel', 'error', 'request-', 'response-', 'event-']) {
      expect(files.some((f) => f.startsWith(prefix))).toBe(true);
    }
  });

  it.each(files)('%s decodes into an envelope', (file) => {
    const envelope = decodeMessagePack(readFileSync(path.join(FIXTURES, file))) as Envelope;
    expect(envelope.wire).toBe(1);
    expect(envelope.session === null || typeof envelope.session === 'number').toBe(true);
    expect(envelope.revision === null || typeof envelope.revision === 'number').toBe(true);
    const name = file.replace(/\.msgpack$/, '');
    const message = envelope.message;
    if (message.kind === 'request') expect(name).toBe(`request-${message.body.type}`);
    else if (message.kind === 'response') expect(name).toBe(`response-${message.body.type}`);
    else if (message.kind === 'event') expect(name.startsWith(`event-${message.body.type}`)).toBe(true);
    else expect(name).toBe(message.kind);
  });

  it('reads the handshake with typed field names', () => {
    const envelope = load('hello');
    expect(envelope.message.kind).toBe('hello');
    const hello = envelope.message as { kind: 'hello' } & Hello;
    expect(hello.role).toBe('renderer');
    expect(hello.contracts).toHaveLength(15);
    expect(hello.contracts[0]).toEqual({ contract: 'evaluatedScene', min: 0, max: 0, required: true });
    expect(hello.maxMessageBytes).toBe(16 << 20);
  });

  it('reads frame requests and descriptors', () => {
    const request = load('request-renderFrame');
    if (request.message.kind !== 'request' || request.message.body.type !== 'renderFrame') throw new Error('wrong fixture');
    const frame: FrameRequest = request.message.body;
    expect(request.revision).toBe(42);
    expect(frame.time).toBe(-29_429_400);
    expect(frame.color).toEqual({ kind: 'viewer', space: 'displayP3' });
    expect(frame.region).toEqual({ x: 0, y: 0, width: 1920, height: 1080 });
    expect(frame.view?.camera?.projection).toEqual({ kind: 'orthographic', halfHeight: 3, near: 0.01, far: 200 });

    const response = load('response-frame');
    if (response.message.kind !== 'response' || response.message.body.type !== 'frame') throw new Error('wrong fixture');
    const descriptor: FrameDescriptor = response.message.body;
    expect(descriptor.seq).toBe(Number.MAX_SAFE_INTEGER);
    expect(descriptor.quality).toEqual({ resolution: 'half', representation: 'proxy', draft3d: false, cached: true });
    expect(descriptor.overlay?.layers[0]?.paths[0]?.[1]).toEqual({ op: 'cubicTo', x1: 10, y1: 0, x2: 20, y2: 10, x: 20, y: 20 });
  });

  it('reads an evaluated scene with separate depth sets and ordering', () => {
    const response = load('response-scene');
    if (response.message.kind !== 'response' || response.message.body.type !== 'scene') throw new Error('wrong fixture');
    const scene: EvaluatedScene = response.message.body;
    expect(scene.frameRate).toEqual({ num: 30000, den: 1001 });
    expect(scene.layers.map((l) => l.id)).toEqual(['bg', 'cube', 'sun', 'title']);
    expect(scene.layers[3]?.compat).toBe(2);
    expect(scene.layers[3]?.effects[0]?.params[0]).toEqual({ key: 'radius', value: { kind: 'number', value: 12.5 } });
    expect(scene.depthSets[0]?.layers).toEqual(['cube']);
    expect(scene.ordering[0]).toEqual({ start: 1, order: ['sun', 'cube'] });
  });

  it('carries bytes as Uint8Array and handles as hex strings', () => {
    const batch = load('event-applyPatches');
    if (batch.message.kind !== 'event' || batch.message.body.type !== 'applyPatches') throw new Error('wrong fixture');
    expect(batch.message.body.body).toBeInstanceOf(Uint8Array);
    expect(Array.from(batch.message.body.body)).toEqual([0x92, 0x01, 0x02]);
    const resync = load('request-resync');
    if (resync.message.kind !== 'request' || resync.message.body.type !== 'resync') throw new Error('wrong fixture');
    expect(resync.message.body.hash).toBeInstanceOf(Uint8Array);
    expect(resync.message.body.hash).toHaveLength(32);
    expect(resync.message.body.source).toEqual({ kind: 'cacheFile', handle: '06'.repeat(16), length: 123_456_789 });
  });

  it('reads errors with their isolation scope', () => {
    const envelope = load('error');
    if (envelope.message.kind !== 'error') throw new Error('wrong fixture');
    expect(envelope.message.error.code).toBe('staleRevision');
    expect(envelope.message.error.scope).toEqual({ kind: 'request', id: 5 });
    expect(envelope.message.error.retry).toEqual({ kind: 'afterResync' });
    expect(envelope.message.error.detail).toEqual({ kind: 'staleRevision', applied: 40, wanted: 42 });
  });

  it('rejects malformed input', () => {
    const bytes = readFileSync(path.join(FIXTURES, 'hello.msgpack'));
    expect(() => decodeMessagePack(bytes.subarray(0, bytes.length - 1))).toThrow();
    expect(() => decodeMessagePack(new Uint8Array([...bytes, 0xc0]))).toThrow(/trailing/);
    expect(() => decodeMessagePack(new Uint8Array([0xcf, 0x00, 0x20, 0, 0, 0, 0, 0, 0]))).toThrow(/unsafe/);
  });
});
