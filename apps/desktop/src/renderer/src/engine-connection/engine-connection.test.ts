/*
 * The kernel's engine connection against a scripted interactive engine:
 * the handshake, a viewer session that resyncs the window's document and
 * then follows pm-wasm's patch stream, resyncs after a gap or a refused
 * batch, capability routing, the transport driver, and an engine restart.
 */
import { describe, expect, it, vi } from 'vitest';

import { PROTOCOL_BUILD, type Envelope, type Request } from '../../../shared/engine-protocol';
import { decodeEnvelope, encodeEnvelope, envelope, localRanges } from '../../../main/engine/protocol';
import { ENGINE_VIEWER_CHANGED as VIEWER_CHANGED, ENGINE_VIEWER_SERVICE as VIEWER_SERVICE } from '../../../extensions/viewer/engine-present/presentation';
import { EngineConnection, type PortLike } from './connection';
import { bestOffset, HostClock } from './host-clock';
import { ENGINE_VIEWER_CHANGED, ENGINE_VIEWER_SERVICE } from './install';
import { DocumentSession, documentId, hexBytes, type StreamBatch } from './session';
import { EngineTransport, mediaTicksAt, ticks, TICKS_PER_SECOND } from './transport';

/** A port pair whose far end is a scripted engine. */
class FakePort implements PortLike {
  sent: Envelope[] = [];
  private listeners: { message: ((e: { data: unknown }) => void)[]; close: (() => void)[] } = { message: [], close: [] };
  closed = false;
  constructor(private readonly reply: (message: Envelope, port: FakePort) => void) {}
  postMessage(message: unknown): void {
    const decoded = decodeEnvelope(message as Uint8Array);
    this.sent.push(decoded);
    this.reply(decoded, this);
  }
  addEventListener(type: 'message' | 'close', listener: never): void {
    (this.listeners[type] as unknown[]).push(listener);
  }
  start(): void {}
  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const l of this.listeners.close) l();
  }
  /** The engine sends `message`. */
  deliver(message: Envelope): void {
    for (const l of this.listeners.message) l({ data: encodeEnvelope(message) });
  }
  requests(type: string): Envelope[] {
    return this.sent.filter((e) => e.message.kind === 'request' && (e.message.body as { type: string }).type === type);
  }
  events(type: string): Envelope[] {
    return this.sent.filter((e) => e.message.kind === 'event' && (e.message.body as { type: string }).type === type);
  }
}

/** An engine that welcomes, opens session 5, and acknowledges resyncs. */
function engine(options: { refuseResync?: boolean } = {}): FakePort {
  return new FakePort((message, port) => {
    const m = message.message;
    if (m.kind === 'hello') {
      queueMicrotask(() =>
        port.deliver(envelope({ kind: 'welcome', role: 'interactiveEngine', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges().map((r) => ({ contract: r.contract, version: 0 })), maxMessageBytes: 16 * 1024 * 1024 } as never))
      );
    } else if (m.kind === 'request') {
      const body = m.body as Request;
      if (body.type === 'openSession') queueMicrotask(() => port.deliver(envelope({ kind: 'response', id: m.id, body: { type: 'sessionOpened', session: 5 } } as never)));
      else if (body.type === 'resync') {
        queueMicrotask(() =>
          port.deliver(
            options.refuseResync
              ? envelope({ kind: 'error', id: m.id, error: { code: 'invalidMessage', scope: { kind: 'request', id: m.id }, message: 'no', retry: { kind: 'never' }, detail: null } } as never)
              : envelope({ kind: 'response', id: m.id, body: { type: 'patchesApplied', document: body.document, revision: body.revision } } as never, 5, body.revision)
          )
        );
      } else if (body.type === 'closeSession') queueMicrotask(() => port.deliver(envelope({ kind: 'response', id: m.id, body: { type: 'ack' } } as never)));
    }
  });
}

/** pm-wasm's stream as a test double. */
function stream() {
  const subscribers = new Set<(batch: StreamBatch) => void>();
  let revision = 3;
  return {
    ready: true,
    whenReady: Promise.resolve(true),
    subscribe(fn: (batch: StreamBatch) => void) {
      subscribers.add(fn);
      return () => subscribers.delete(fn);
    },
    snapshot: () => ({ revision, encoding: 'canonicalJsonV1' as const, json: `{"id":"P","rev":${revision}}`, hash: 'ab'.repeat(32) }),
    emit(batch: Partial<StreamBatch> & { revision: number }) {
      const full: StreamBatch = { baseRevision: revision, origin: 'edit', count: 1, body: new Uint8Array([1, 2, 3]), ...batch };
      revision = full.revision;
      for (const fn of subscribers) fn(full);
    },
    set(next: number) {
      revision = next;
    }
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

async function connected(port = engine()) {
  const connection = new EngineConnection(1, port, () => {});
  await connection.open();
  return { connection, port };
}

describe('the engine connection', () => {
  it('shares the viewer service names with the viewer', () => {
    expect(ENGINE_VIEWER_SERVICE).toBe(VIEWER_SERVICE);
    expect(ENGINE_VIEWER_CHANGED).toBe(VIEWER_CHANGED);
  });

  it('negotiates as a renderer and refuses a peer that is not the interactive engine', async () => {
    const { connection, port } = await connected();
    expect(connection.ready).toBe(true);
    expect(port.sent[0]!.message).toMatchObject({ kind: 'hello', role: 'renderer', build: PROTOCOL_BUILD });
    const wrong = new FakePort((m, p) => {
      if (m.message.kind === 'hello') queueMicrotask(() => p.deliver(envelope({ kind: 'welcome', role: 'jobEngine', wire: 1, build: PROTOCOL_BUILD, contracts: [], maxMessageBytes: 1 << 24 } as never)));
    });
    await expect(new EngineConnection(1, wrong, () => {}).open()).rejects.toThrow(/interactiveEngine/);
  });

  it('opens a session, resyncs the document, and follows the patch stream in order', async () => {
    const { connection, port } = await connected();
    const doc = stream();
    const messages: string[] = [];
    const routes: string[] = [];
    const session = new DocumentSession({
      connection, stream: doc, documentId: () => 'Pproject', log: () => {},
      handlers: { message: (m) => messages.push(m.type), closed: () => messages.push('closed') },
      onRoute: (route) => routes.push(route)
    });
    session.sendViewerState({ composition: 'c', viewSeq: 1 } as never);
    await session.start();
    await flush();
    const resync = port.requests('resync')[0]!;
    const body = (resync.message as { body: { document: string; revision: number; source: { kind: string; body: Uint8Array }; hash: Uint8Array } }).body;
    expect(body.document).toBe('Pproject');
    expect(body.revision).toBe(3);
    expect(new TextDecoder().decode(body.source.body)).toBe('{"id":"P","rev":3}');
    expect(Array.from(body.hash)).toEqual(Array.from(hexBytes('ab'.repeat(32))));
    // The viewer state waited for the session and its document.
    expect(port.events('viewer')).toHaveLength(1);
    doc.emit({ revision: 4 });
    doc.emit({ revision: 5 });
    const applied = port.events('applyPatches').map((e) => (e.message as { body: { baseRevision: number; revision: number } }).body);
    expect(applied.map((b) => [b.baseRevision, b.revision])).toEqual([[3, 4], [4, 5]]);
    expect(session.batches).toBe(2);

    // A frame says the engine draws the composition; an `unsupported`
    // failure routes it to the legacy viewer.
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 0, revision: 5 } } as never, 5));
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 6, error: { code: 'unsupported', scope: { kind: 'session', session: 5 }, message: 'Drawn by the current engine: effects.', retry: { kind: 'never' }, detail: null } } } as never, 5));
    expect(routes).toEqual(['engine', 'legacy']);
    expect(messages).toEqual(['frame', 'routed']);
    expect(session.routeReason).toMatch(/effects/);
  });

  it('resyncs after a revision gap, a load batch, and a stale failure', async () => {
    const { connection, port } = await connected();
    const doc = stream();
    const session = new DocumentSession({ connection, stream: doc, documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    await flush();
    expect(port.requests('resync')).toHaveLength(1);
    // A batch we never saw sits between.
    doc.emit({ revision: 6, baseRevision: 5 });
    await flush();
    expect(port.requests('resync')).toHaveLength(2);
    // Another project opened.
    doc.emit({ revision: 7, origin: 'load' });
    await flush();
    expect(port.requests('resync')).toHaveLength(3);
    // The engine could not place a batch.
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 7, error: { code: 'staleRevision', scope: { kind: 'session', session: 5 }, message: 'gap', retry: { kind: 'afterResync' }, detail: null } } } as never, 5));
    await flush();
    expect(port.requests('resync')).toHaveLength(4);
    expect(session.resyncs).toBe(4);
  });

  it('asks the viewer for its state again once another document is loaded', async () => {
    const { connection, port } = await connected();
    const doc = stream();
    let id = 'P1';
    const loaded: number[] = [];
    const session = new DocumentSession({
      connection, stream: doc, documentId: () => id, log: () => {},
      handlers: { message: () => {}, closed: () => {} },
      onLoaded: () => loaded.push(port.requests('resync').length)
    });
    await session.start();
    await flush();
    // Edits and gaps keep the document: no new layout.
    doc.emit({ revision: 4 });
    doc.emit({ revision: 6, baseRevision: 5 });
    await flush();
    expect(loaded).toEqual([]);
    // `PM.proj = PM.mkProject(…)`: the mirror loads another document. The
    // viewer is asked again only after the engine has it.
    id = 'P2';
    doc.emit({ revision: 7, origin: 'load' });
    expect(loaded).toEqual([]);
    await flush();
    const last = port.requests('resync').at(-1)!.message as { body: { document: string } };
    expect(last.body.document).toBe('P2');
    expect(loaded).toEqual([3]);
    expect(session.applied).toBe(7);
    // A closed session asks for nothing.
    session.close();
    doc.emit({ revision: 8, origin: 'load' });
    await flush();
    expect(loaded).toEqual([3]);
  });

  it('ends its sessions when the engine connection closes (an engine restart)', async () => {
    const { connection, port } = await connected();
    const closed = vi.fn();
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed } });
    await session.start();
    port.close();
    expect(closed).toHaveBeenCalledWith('the engine port closed');
    expect(session.closed).toBe(true);
    expect(connection.closed).toBe(true);
  });

  it('stages a large document as a cache file', async () => {
    const { connection, port } = await connected();
    const stage = vi.fn(async (_session: number, bytes: Uint8Array) => ({ handle: 'f'.repeat(32), length: bytes.length }));
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: () => {}, stage, inlineLimit: 4, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    expect(stage).toHaveBeenCalledWith(5, expect.any(Uint8Array));
    const source = (port.requests('resync')[0]!.message as { body: { source: { kind: string; handle: string } } }).body.source;
    expect(source).toEqual({ kind: 'cacheFile', handle: 'f'.repeat(32), length: 18 });
  });

  it('keeps document ids within the contract', () => {
    expect(documentId('')).toBe('untitled');
    expect(documentId('x'.repeat(300))).toHaveLength(64);
    expect(documentId('P123')).toBe('P123');
  });
});

describe('the transport driver', () => {
  it('runs the transport through the engine and follows its clock', async () => {
    const { connection, port } = await connected();
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    const clock = new HostClock(async () => 0);
    (clock as unknown as { best: { offsetUs: number; rttUs: number } }).best = { offsetUs: 1_000_000, rttUs: 10 };
    const driver = new EngineTransport(() => session, clock);
    const changes: boolean[] = [];
    driver.onActiveChange((active) => changes.push(active));
    expect(driver.active()).toBe(false);
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 0, revision: 3 } } as never, 5));
    driver.refresh();
    expect(driver.active()).toBe(true);
    expect(changes).toEqual([true]);

    driver.setLoop([0, 2]);
    driver.setLoop([0, 2]);
    driver.play(1.5);
    const commands = port.events('transport').map((e) => (e.message as { body: { command: string } }).body);
    expect(commands).toEqual([
      { type: 'transport', command: 'setLoop', range: { start: 0, end: 2 * TICKS_PER_SECOND } },
      { type: 'transport', command: 'seek', time: ticks(1.5), mode: 'exact' },
      { type: 'transport', command: 'play', rate: { num: 1, den: 1 } }
    ]);
    // Before the audio's first playing sample the playhead stays.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 1, state: 'stopped', hostTime: 0, mediaTime: ticks(1.5), rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.now(0)).toEqual({ time: 1.5, playing: false, ended: false });
    // Playing: heard at host 2 s (page 1000 ms with the 1 s offset).
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 2, epoch: 2, state: 'playing', hostTime: 2_000_000, mediaTime: ticks(1.5), rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.now(1250)!.time).toBeCloseTo(1.75, 9);
    // Until a run's first sample is heard (its host time is still ahead:
    // the output latency) the playhead stays at the start, never behind it.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 3, state: 'playing', hostTime: 3_160_000, mediaTime: ticks(1.5), rate: { num: 1, den: 1 }, outputLatencyUs: 160_000, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.now(2000)).toEqual({ time: 1.5, playing: true, ended: false });
    expect(driver.now(2260)!.time).toBeCloseTo(1.6, 9);
    // The run ends at the composition's end.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 2, epoch: 3, state: 'stopped', hostTime: 9_000_000, mediaTime: ticks(2), rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.now(9000)).toEqual({ time: 2, playing: false, ended: true });

    // Routed to the legacy viewer: the driver hands back.
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 4, error: { code: 'unsupported', scope: { kind: 'session', session: 5 }, message: 'masks', retry: { kind: 'never' }, detail: null } } } as never, 5));
    driver.refresh();
    expect(changes).toEqual([true, false]);
  });

  it('wraps a looping line as contract 6 does', () => {
    const sample = { seq: 1, epoch: 1, state: 'playing', hostTime: 0, mediaTime: ticks(1), rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: { start: ticks(1), end: ticks(2) } } as const;
    expect(mediaTicksAt(sample as never, 1_500_000)).toBe(ticks(1.5));
    expect(mediaTicksAt(sample as never, 2_250_000)).toBe(ticks(1.25));
  });
});

describe('the host clock', () => {
  it('keeps the offset of the fastest round trip', async () => {
    let page = 0;
    const host = vi.fn(async () => {
      page += 0.4; // 400 µs round trip
      return 5_000_000 + page * 1000;
    });
    const clock = new HostClock(host, () => page);
    await clock.measure(4);
    expect(clock.offsetUs()).not.toBeNull();
    expect(clock.errorUs()).toBeLessThanOrEqual(200);
    expect(bestOffset([{ offsetUs: 1, rttUs: 5 }, { offsetUs: 2, rttUs: 1 }])).toEqual({ offsetUs: 2, rttUs: 1 });
    expect(clock.pageMs(clock.hostUs(10)!)).toBeCloseTo(10, 9);
  });
});
