// Main's MessagePack codec and protocol helpers against the golden fixtures
// written by engine/crates/pm-protocol (the same bytes the Rust engine
// produces and accepts).
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import { PROTOCOL_BUILD, type Envelope, type Welcome } from '../../shared/engine-protocol';
import { decode, encode, MessagePackError } from './msgpack';
import { completeWelcome, decodeEnvelope, encodeEnvelope, hostHello, localRanges, Outstanding, ProtocolError, RemoteError } from './protocol';

const FIXTURES = path.resolve(__dirname, '../../../../../engine/crates/pm-protocol/tests/fixtures/wire');
const fixtures = readdirSync(FIXTURES).filter((f) => f.endsWith('.msgpack')).sort();
const read = (name: string): Uint8Array => new Uint8Array(readFileSync(path.join(FIXTURES, name)));

/** Messages main writes itself: integers, strings, and booleans only, so
 *  main's encoder must reproduce rmp_serde's bytes exactly. */
const HOST_WRITTEN = [
  'hello', 'request-grantLease', 'response-lease', 'response-handle', 'response-handleResolved',
  'response-outputDestination', 'response-outputCommitted', 'response-hardwareSessions',
  'event-slicing', 'event-crash', 'event-deadline-deadlineMiss', 'request-closeSession', 'cancel', 'error'
];

describe('MessagePack', () => {
  it('decodes every golden fixture and re-encodes it to an equal value', () => {
    expect(fixtures.length).toBeGreaterThan(60);
    for (const name of fixtures) {
      const value = decode(read(name));
      expect(decode(encode(value)), name).toEqual(value);
    }
  });

  it('writes the same bytes as rmp_serde for the messages main sends', () => {
    for (const name of HOST_WRITTEN) {
      const bytes = read(`${name}.msgpack`);
      expect(Buffer.from(encode(decode(bytes))).toString('hex'), name).toBe(Buffer.from(bytes).toString('hex'));
    }
  });

  it('is strict about what it accepts', () => {
    const bad: Array<[string, number[]]> = [
      ['duplicate key', [0x82, 0xa1, 0x61, 0x01, 0xa1, 0x61, 0x02]],
      ['non-string key', [0x81, 0x01, 0x01]],
      ['unsafe integer', [0xcf, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]],
      ['NaN', [0xcb, 0x7f, 0xf8, 0, 0, 0, 0, 0, 0]],
      ['infinity', [0xca, 0x7f, 0x80, 0x00, 0x00]],
      ['trailing bytes', [0xc0, 0xc0]],
      ['truncated', [0xa5, 0x61]],
      ['invalid UTF-8', [0xa2, 0xc3, 0x28]],
      ['ext type', [0xd4, 0x01, 0x00]],
      ['huge array header', [0xdd, 0xff, 0xff, 0xff, 0xff]]
    ];
    for (const [label, bytes] of bad) expect(() => decode(new Uint8Array(bytes)), label).toThrow(MessagePackError);
    expect(() => decode(new Uint8Array([...Array(80).fill(0x91), 0xc0]))).toThrow(/nesting/);
    // `__proto__` is a plain key, never a prototype.
    const value = decode(new Uint8Array([0x81, 0xa9, ...Buffer.from('__proto__'), 0x01])) as Record<string, unknown>;
    expect(Object.getPrototypeOf(value)).toBe(null);
    expect(value['__proto__']).toBe(1);
  });

  it('encodes JavaScript numbers in their smallest form and refuses the rest', () => {
    const hex = (v: unknown) => Buffer.from(encode(v)).toString('hex');
    expect(hex(0)).toBe('00');
    expect(hex(-32)).toBe('e0');
    expect(hex(255)).toBe('ccff');
    expect(hex(-129)).toBe('d1ff7f');
    expect(hex(2 ** 32)).toBe('cf0000000100000000');
    expect(hex(-(2 ** 40))).toBe('d3ffffff0000000000');
    expect(hex(0.5)).toBe('cb3fe0000000000000');
    expect(hex(new Uint8Array([1, 2]))).toBe('c4020102');
    expect(() => encode(Number.NaN)).toThrow();
    expect(() => encode(2 ** 60)).toThrow();
    expect(() => encode(() => 1)).toThrow();
    for (const n of [1, -1, 127, 128, -33, 65535, 65536, -32769, 2 ** 31, -(2 ** 31) - 1, Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER]) {
      expect(decode(encode(n))).toBe(n);
    }
  });

  it('survives random and mutated input without anything but MessagePackError', () => {
    let seed = 0x2545f491;
    const next = () => {
      seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
      return seed >>> 0;
    };
    const samples = fixtures.map(read);
    for (let i = 0; i < 2048; i++) {
      const base = samples[i % samples.length]!;
      const bytes = base.slice();
      bytes[next() % bytes.length] = next() & 0xff;
      try { decode(bytes); } catch (error) { expect(error).toBeInstanceOf(MessagePackError); }
      const random = new Uint8Array(1 + (next() % 64)).map(() => next() & 0xff);
      try { decode(random); } catch (error) { expect(error).toBeInstanceOf(MessagePackError); }
    }
  });
});

describe('envelopes and negotiation', () => {
  it('reads every fixture as an envelope', () => {
    for (const name of fixtures) {
      const envelope = decodeEnvelope(read(name));
      expect(envelope.wire).toBe(1);
      expect(decodeEnvelope(encodeEnvelope(envelope))).toEqual(envelope);
    }
    expect(() => decodeEnvelope(encode({ wire: 2, session: null, revision: null, message: { kind: 'cancel', id: 1 } }))).toThrow(ProtocolError);
    expect(() => decodeEnvelope(encode({ wire: 1, session: -1, revision: null, message: { kind: 'cancel', id: 1 } }))).toThrow(ProtocolError);
    expect(() => decodeEnvelope(encode({ wire: 1, session: null, revision: null, message: { kind: 'request', id: 1, body: {} } }))).toThrow(ProtocolError);
  });

  it('sends the hello the Rust fixture describes, with this build', () => {
    const hello = hostHello();
    expect(hello.build).toBe(PROTOCOL_BUILD);
    expect(hello.contracts).toHaveLength(15);
    const fixture = decodeEnvelope(read('hello.msgpack')).message;
    expect(fixture.kind).toBe('hello');
    if (fixture.kind === 'hello') expect(Object.keys(fixture).sort()).toEqual(Object.keys({ kind: 'hello', ...hello }).sort());
  });

  it('checks the welcome like pm_protocol::version::complete', () => {
    const welcome: Welcome = {
      role: 'jobEngine',
      wire: 1,
      build: PROTOCOL_BUILD,
      contracts: localRanges().map((r) => ({ contract: r.contract, version: 0 })),
      maxMessageBytes: 1 << 20
    };
    const negotiated = completeWelcome(welcome, 'jobEngine');
    expect(negotiated.versions.size).toBe(15);
    expect(negotiated.maxMessageBytes).toBe(1 << 20);
    expect(() => completeWelcome(welcome, 'interactiveEngine')).toThrow(/expected/);
    expect(() => completeWelcome({ ...welcome, build: 'protocol-0000000000000000' }, 'jobEngine')).toThrow(/equal builds/);
    expect(() => completeWelcome({ ...welcome, wire: 2 }, 'jobEngine')).toThrow(/wire/);
    expect(() => completeWelcome({ ...welcome, maxMessageBytes: 1024 }, 'jobEngine')).toThrow(/64 KiB/);
    expect(() => completeWelcome({ ...welcome, contracts: welcome.contracts.slice(1) }, 'jobEngine')).toThrow(/required/);
    expect(() => completeWelcome({ ...welcome, contracts: [...welcome.contracts, welcome.contracts[0]!] }, 'jobEngine')).toThrow(/twice/);
    expect(() => completeWelcome({ ...welcome, contracts: [{ contract: 'lease', version: 3 }] }, 'jobEngine')).toThrow(/outside/);
  });

  it('correlates answers by id and kind', async () => {
    const outstanding = new Outstanding();
    const lease = outstanding.open({ type: 'grantLease', process: 'job', epoch: 1, bytes: 10, reason: 'initial', ackDeadlineMs: 500 });
    const close = outstanding.open({ type: 'closeSession' });
    expect(outstanding.settle({ kind: 'response', id: 99, body: { type: 'ack' } })).toBe(false);
    expect(outstanding.settle({ kind: 'response', id: close.id, body: { type: 'ack' } })).toBe(true);
    await expect(close.answer).resolves.toEqual({ type: 'ack' });
    expect(outstanding.settle({ kind: 'response', id: lease.id, body: { type: 'ack' } })).toBe(true);
    await expect(lease.answer).rejects.toThrow(/expected leaseUsage/);
    const failing = outstanding.open({ type: 'closeSession' });
    outstanding.settle({ kind: 'error', id: failing.id, error: { code: 'unknownSession', scope: { kind: 'connection' }, message: 'gone', retry: { kind: 'never' }, detail: null } });
    await expect(failing.answer).rejects.toBeInstanceOf(RemoteError);
    const pending = outstanding.open({ type: 'closeSession' });
    outstanding.failAll(new Error('closed'));
    await expect(pending.answer).rejects.toThrow('closed');
    expect(outstanding.size).toBe(0);
  });

  it('round-trips the envelope shape main builds', () => {
    const envelope: Envelope = { wire: 1, session: 3, revision: 9, message: { kind: 'event', body: { type: 'slicing', maxCommandBufferUs: 2000, targetCommandBufferUs: 1000, maxInFlight: 1, alignGuardUs: 1000, yieldAfterMissUs: 100000, blender: { kind: 'pause' } } } };
    expect(decodeEnvelope(encodeEnvelope(envelope))).toEqual(envelope);
  });
});
