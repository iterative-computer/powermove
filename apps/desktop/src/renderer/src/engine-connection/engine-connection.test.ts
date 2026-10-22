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
import { ENGINE_VIEWER_CHANGED, ENGINE_VIEWER_SERVICE, missedRoute } from './install';
import { DEFINITIONS_RETRIES, DocumentSession, documentId, hexBytes, type StreamBatch } from './session';
import { EngineTransport, mediaTicksAt, ticks, TICKS_PER_SECOND } from './transport';
import { followOpenComposition } from './composition-follow';
import { EnginePreview, documentWake } from './preview';
import { createEngineDefinitions } from '../kernel/engine-definitions';
import { createKernel } from '../kernel/registries';

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
function engine(options: { refuseResync?: boolean | (() => boolean) } = {}): FakePort {
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
            (typeof options.refuseResync === 'function' ? options.refuseResync() : options.refuseResync)
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
    await flush();
    doc.emit({ revision: 5 });
    await flush();
    const applied = port.events('applyPatches').map((e) => (e.message as { body: { baseRevision: number; revision: number } }).body);
    expect(applied.map((b) => [b.baseRevision, b.revision])).toEqual([[3, 4], [4, 5]]);
    expect(session.batches).toBe(2);

    // A frame says the engine draws the composition; an `unsupported`
    // failure routes it to the legacy viewer. The report names the newest
    // frame seq the engine had given the view: 2, a paused frame of
    // revision 5 the GPU had not finished yet (the completion thread sends
    // it), so it was never received.
    const failure = (revision: number, lastFrame: number | null) => envelope({ kind: 'event', body: { type: 'failure', revision, error: { code: 'unsupported', scope: { kind: 'session', session: 5 }, message: 'Drawn by the current engine: effects.', retry: { kind: 'never' }, detail: lastFrame === null ? null : { kind: 'unsupported', lastFrame } } } } as never, 5);
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 0, seq: 1, revision: 5 } } as never, 5));
    port.deliver(failure(6, 2));
    expect(routes).toEqual(['engine', 'legacy']);
    expect(messages).toEqual(['frame', 'routed']);
    expect(session.routeReason).toMatch(/effects/);
    // That frame, still in flight at the report, arrives after it with a
    // higher seq than any frame received: it is of the older revision and
    // stale. It neither shows nor brings the engine back.
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 1, seq: 2, revision: 5 } } as never, 5));
    expect(routes).toEqual(['engine', 'legacy']);
    expect(messages).toEqual(['frame', 'routed']);
    expect(session.routeReason).toMatch(/effects/);
    expect(session.lastFrame?.seq).toBe(1);
    // A composition switch: the document batch reached the engine before the
    // viewer state, so it reported the old composition at revision 6 and
    // then drew the new one at the same revision. That frame is a later
    // decision and brings the engine back.
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 0, seq: 3, revision: 6 } } as never, 5));
    expect(routes).toEqual(['engine', 'legacy', 'engine']);
    expect(messages).toEqual(['frame', 'routed', 'frame']);
    expect(session.routeReason).toBeNull();
    // Back to the composition the engine cannot draw and over again (A, B,
    // A, B), every report and frame at revision 7. This report carries no
    // seq (the gate falls back to the newest frame received).
    port.deliver(failure(7, null));
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 0, seq: 3, revision: 6 } } as never, 5));
    expect(routes).toEqual(['engine', 'legacy', 'engine', 'legacy']);
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 1, seq: 4, revision: 7 } } as never, 5));
    expect(routes).toEqual(['engine', 'legacy', 'engine', 'legacy', 'engine']);
    expect(messages).toEqual(['frame', 'routed', 'frame', 'routed', 'frame']);
    expect(session.lastFrame?.seq).toBe(4);
  });

  it('keeps the session open for a later resync when the initial resync is refused', async () => {
    const { connection, port } = await connected(engine({ refuseResync: true }));
    const logs: string[] = [];
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: (_level, text) => logs.push(text), handlers: { message: () => {}, closed: () => {} } });
    session.sendHeadless('comp');
    await expect(session.start()).resolves.toBeUndefined();
    expect(session.resyncs).toBe(0);
    expect(session.documentAccepted).toBe(false);
    expect(session.closed).toBe(false);
    expect(port.requests('resync')).toHaveLength(1);
    expect(logs.some(text => text.includes('engine resync (open) failed'))).toBe(true);
    session.close(); connection.close('test complete');
  });

  it('tracks acceptance of the current document across refused loads and recovery', async () => {
    let refuse = true;
    const { connection, port } = await connected(engine({ refuseResync: () => refuse }));
    const doc = stream();
    const session = new DocumentSession({ connection, stream: doc, documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    expect(session.documentAccepted).toBe(false);
    expect(session.resyncs).toBe(0);

    refuse = false;
    await session.resync('retry');
    expect(session.documentAccepted).toBe(true);
    expect(session.resyncs).toBe(1);

    refuse = true;
    doc.emit({ revision: 4, origin: 'load' });
    await flush();
    expect(port.requests('resync')).toHaveLength(3);
    expect(session.resyncs).toBe(1);
    expect(session.documentAccepted).toBe(false);
    const preview = new EnginePreview(
      () => session, () => 'P', () => 0, () => 4,
      undefined, documentWake(async () => {}, () => session),
    );
    await expect(preview.render({ cacheOnly: true, size: { width: 32, height: 32 } }))
      .rejects.toMatchObject({ name: 'EnginePreviewUnavailable' });
    expect(port.events('previewControl')).toHaveLength(0);

    refuse = false;
    await session.resync('retry load');
    expect(session.resyncs).toBe(2);
    expect(session.documentAccepted).toBe(true);
    const rendered = preview.render({ cacheOnly: true, size: { width: 32, height: 32 } });
    await flush();
    expect(port.events('previewControl')).toHaveLength(1);
    preview.receiveStatus(session, { generation: 1, automatic: false, revision: 4, composition: 'P', time: 0, phase: 'ready', frameSeq: null, error: null, cacheOnly: true });
    await rendered;
    session.close(); connection.close('test complete');
  });

  it('streams the kernel\'s definitions to the session before its document, and their failures back', async () => {
    const pushes: Array<{ generation: number; full: boolean; ids: string[] }> = [];
    let refuseNext = false;
    const base = engine();
    const port = new FakePort((message, p) => {
      const m = message.message;
      if (m.kind === 'request' && (m.body as Request).type === 'pushDefinitions') {
        const body = m.body as unknown as { generation: number; full: boolean; upserts: Array<{ id: string }> };
        pushes.push({ generation: body.generation, full: body.full, ids: body.upserts.map((u) => u.id) });
        const refuse = refuseNext;
        refuseNext = false;
        queueMicrotask(() =>
          p.deliver(
            refuse
              ? envelope({ kind: 'error', id: m.id, error: { code: 'invalidMessage', scope: { kind: 'request', id: m.id }, message: 'gap', retry: { kind: 'afterResync' }, detail: null } } as never)
              : envelope({ kind: 'response', id: m.id, body: { type: 'definitionsApplied', generation: body.generation, failures: body.upserts.some((u) => u.id === 'broken') ? [{ code: 'shaderCompile', scope: { kind: 'definition', definition: 'broken' }, message: 'The shader of broken does not compile.', retry: { kind: 'afterUserAction' }, detail: { kind: 'shaderCompile', diagnostics: [{ line: 2, column: null, message: 'syntax error' }] } }] : [] } } as never, 5)
          )
        );
        return;
      }
      (base as unknown as { reply: (m: Envelope, p: FakePort) => void }).reply(message, p);
    });
    const { connection } = await connected(port);
    const kernel = createKernel();
    const PM: any = { GLSL_PRE: '#version 300 es\nprecision highp float;\nout vec4 o;', TRANSITION_PRE: '#version 300 es\nprecision highp float;\nout vec4 o;', SHADER_HEADER: '#version 300 es', proj: { layers: [], comps: {} }, UIState: { setShaderMeta: () => {} } };
    kernel.registerEffect('local', { id: 'glow2', label: 'Glow', group: 'Light', params: [], frag: 'o = vec4(1.0);' });
    const definitions = createEngineDefinitions(PM, kernel);
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} }, definitions: () => definitions });
    await session.start();
    await flush();
    // The full update went before the document.
    const order = port.sent.flatMap((e) => (e.message.kind === 'request' ? [(e.message.body as { type: string }).type] : []));
    expect(order.indexOf('pushDefinitions')).toBeLessThan(order.indexOf('resync'));
    expect(pushes).toEqual([{ generation: 1, full: true, ids: ['glow2'] }]);
    // A registration is a delta; its compile failure comes back as a compile error.
    kernel.registerEffect('local', { id: 'broken', label: 'Broken', group: 'X', params: [], frag: 'o = ;' });
    await flush();
    expect(pushes.at(-1)).toEqual({ generation: 2, full: false, ids: ['broken'] });
    expect(definitions.meta.compileError('fx:broken')).toBe('The shader of broken does not compile.\nline 2: syntax error');
    // A refused delta (an engine restart lost the generation): the next update is full.
    refuseNext = true;
    kernel.registerEffect('local', { id: 'third', label: 'Third', group: 'X', params: [], frag: 'o = vec4(0.0);' });
    await flush();
    await flush();
    expect(pushes.at(-2)).toEqual({ generation: 3, full: false, ids: ['third'] });
    expect(pushes.at(-1)).toMatchObject({ generation: 4, full: true });
    expect(pushes.at(-1)!.ids.sort()).toEqual(['broken', 'glow2', 'third']);
    // Failure events reach the shader meta.
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 9, error: { code: 'watchdogTimeout', scope: { kind: 'effect', layer: 'L', effect: 'f', definition: 'glow2' }, message: 'glow2 took 300 ms.', retry: { kind: 'afterUserAction' }, detail: null } } } as never, 5));
    expect(definitions.meta.compileError('fx:glow2')).toBeNull(); // no layer L in the document: ignored
    session.close();
    kernel.registerEffect('local', { id: 'late', label: 'Late', group: 'X', params: [], frag: 'o = vec4(0.0);' });
    await flush();
    expect(pushes.at(-1)!.ids).not.toContain('late');
  });

  it('backs off from an engine that keeps refusing definitions, then waits for the next registration', async () => {
    const pushes: Array<{ at: number; full: boolean }> = [];
    let refuse = true;
    const base = engine();
    const port = new FakePort((message, p) => {
      const m = message.message;
      if (m.kind === 'request' && (m.body as Request).type === 'pushDefinitions') {
        const body = m.body as unknown as { generation: number; full: boolean };
        pushes.push({ at: Date.now(), full: body.full });
        const refused = refuse;
        queueMicrotask(() =>
          p.deliver(
            refused
              ? envelope({ kind: 'error', id: m.id, error: { code: 'invalidMessage', scope: { kind: 'request', id: m.id }, message: 'bad snapshot', retry: { kind: 'afterResync' }, detail: null } } as never)
              : envelope({ kind: 'response', id: m.id, body: { type: 'definitionsApplied', generation: body.generation, failures: [] } } as never, 5)
          )
        );
        return;
      }
      (base as unknown as { reply: (m: Envelope, p: FakePort) => void }).reply(message, p);
    });
    const { connection } = await connected(port);
    const kernel = createKernel();
    const PM: any = { GLSL_PRE: '#version 300 es\nprecision highp float;\nout vec4 o;', TRANSITION_PRE: '', SHADER_HEADER: '', proj: { layers: [], comps: {} } };
    kernel.registerEffect('local', { id: 'glow2', label: 'Glow', group: 'Light', params: [], frag: 'o = vec4(1.0);' });
    const definitions = createEngineDefinitions(PM, kernel);
    const logs: string[] = [];
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: (_l, m) => logs.push(m), handlers: { message: () => {}, closed: () => {} }, definitions: () => definitions, definitionsRetryMs: 4 });
    expect(definitions.sessionCount()).toBe(0);
    await session.start();
    expect(definitions.sessionCount()).toBe(1);
    // The first push, an immediate full retry, then DEFINITIONS_RETRIES - 1
    // retries backing off from 4 ms: 4, 8, 16, 32, 64 ms.
    await vi.waitFor(() => expect(logs.some((l) => l.includes('times in a row'))).toBe(true), { timeout: 2000 });
    const settled = pushes.length;
    expect(settled).toBe(1 + DEFINITIONS_RETRIES);
    expect(pushes.every((p) => p.full)).toBe(true);
    const gaps = pushes.slice(2).map((p, i) => p.at - pushes[i + 1]!.at);
    gaps.forEach((gap, i) => expect(gap).toBeGreaterThanOrEqual(4 * 2 ** i - 1));
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(pushes.length).toBe(settled); // no endless loop
    // The next registration sends the whole set again.
    refuse = false;
    kernel.registerEffect('local', { id: 'third', label: 'Third', group: 'X', params: [], frag: 'o = vec4(0.0);' });
    await flush();
    await flush();
    expect(pushes.length).toBe(settled + 1);
    expect(pushes.at(-1)!.full).toBe(true);
    // A closed session leaves the kernel's list.
    session.close();
    expect(definitions.sessionCount()).toBe(0);
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
    const loading = vi.fn();
    const session = new DocumentSession({
      connection, stream: doc, documentId: () => id, log: () => {},
      handlers: { message: () => {}, closed: () => {} },
      onLoading: loading,
      onLoaded: () => loaded.push(port.requests('resync').length)
    });
    await session.start();
    await flush();
    // Edits and gaps keep the document: no new layout.
    doc.emit({ revision: 4 });
    doc.emit({ revision: 6, baseRevision: 5 });
    await flush();
    expect(loaded).toEqual([]);
    expect(loading).not.toHaveBeenCalled();
    // `PM.proj = PM.mkProject(…)`: the mirror loads another document. The
    // viewer is asked again only after the engine has it.
    id = 'P2';
    doc.emit({ revision: 7, origin: 'load' });
    expect(loading).toHaveBeenCalledOnce(); // preserve UI time before asynchronous resync
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

  it('tells a viewer replacing another why the engine draws nothing', () => {
    // Presenting a composition the engine cannot draw: the new viewer is told.
    expect(missedRoute({ headless: false, route: 'legacy', routeReason: 'The new engine cannot draw this composition yet: path masks.' }))
      .toEqual({ type: 'routed', route: 'legacy', reason: 'The new engine cannot draw this composition yet: path masks.' });
    // Viewerless (the engine says it again when a viewer attaches), drawn,
    // or undecided: nothing missed.
    expect(missedRoute({ headless: true, route: 'legacy', routeReason: 'x' })).toBeNull();
    expect(missedRoute({ headless: false, route: 'engine', routeReason: null })).toBeNull();
    expect(missedRoute({ headless: false, route: 'pending', routeReason: null })).toBeNull();
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
      // A session the driver becomes active on learns the audio scrub switch.
      { type: 'transport', command: 'audioScrub', enabled: false },
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

    // A composition the engine cannot draw (amendment 21): nothing hands
    // the transport back; Play still runs on the engine's clock.
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 4, error: { code: 'unsupported', scope: { kind: 'session', session: 5 }, message: 'masks', retry: { kind: 'never' }, detail: null } } } as never, 5));
    driver.refresh();
    expect(session.route).toBe('legacy');
    expect(changes).toEqual([true]);
    expect(driver.active()).toBe(true);
    const before = port.events('transport').length;
    driver.play(0.5);
    const after = port.events('transport').slice(before).map((e) => (e.message as { body: { command: string } }).body.command);
    expect(after).toEqual(['seek', 'play']);
  });

  it('sends J/K/L rates, play around, and the audio scrub switch, and follows a line at any rate', async () => {
    const { connection, port } = await connected();
    const session = new DocumentSession({ connection, stream: stream(), documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    const clock = new HostClock(async () => 0);
    (clock as unknown as { best: { offsetUs: number; rttUs: number } }).best = { offsetUs: 0, rttUs: 10 };
    const driver = new EngineTransport(() => session, clock);
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', poolIndex: 0, revision: 3 } } as never, 5));
    driver.refresh();
    driver.setAudioScrub(true);
    driver.shuttle(4, -1);
    const sent = () => port.events('transport').map((e) => (e.message as { body: Record<string, unknown> }).body);
    expect(sent()).toEqual([
      { type: 'transport', command: 'audioScrub', enabled: false },
      { type: 'transport', command: 'audioScrub', enabled: true },
      { type: 'transport', command: 'seek', time: ticks(4), mode: 'exact' },
      { type: 'transport', command: 'play', rate: { num: -1, den: 1 } }
    ]);
    // Backwards at 2×: the playhead runs back two seconds a second.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 2, state: 'shuttling', hostTime: 1_000_000, mediaTime: ticks(4), rate: { num: -2, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.rate()).toBe(-2);
    expect(driver.now(1500)!.time).toBeCloseTo(3, 9);
    // A new rate while the line moves continues from what is heard: no seek.
    const before = sent().length;
    driver.shuttle(3, 2);
    expect(sent().slice(before)).toEqual([{ type: 'transport', command: 'play', rate: { num: 2, den: 1 } }]);
    driver.playAround(2.5, 2, 1);
    expect(sent().slice(-2)).toEqual([
      { type: 'transport', command: 'seek', time: ticks(2.5), mode: 'exact' },
      { type: 'transport', command: 'playAround', preRoll: ticks(2), postRoll: ticks(1) }
    ]);
    // The play around's run ends back on its frame.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 3, state: 'playing', hostTime: 2_000_000, mediaTime: ticks(0.5), rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.now(2100)!.time).toBeCloseTo(0.6, 9);
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 2, epoch: 4, state: 'stopped', hostTime: 5_000_000, mediaTime: ticks(2.5), rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(driver.now(5100)).toEqual({ time: 2.5, playing: false, ended: true });
    expect(driver.rate()).toBe(0);
  });

  it('wakes a headless session and replays Play without requiring a frame', async () => {
    const { connection, port } = await connected();
    const doc = stream(); doc.set(0);
    const session = new DocumentSession({ connection, stream: doc, documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    session.sendHeadless('C');
    const wake = vi.fn();
    const driver = new EngineTransport(() => session, new HostClock(async () => 0), { exclusive: true, wake });
    driver.setLoop([0.2, 1.2]);
    driver.play(0.5);
    expect(wake).toHaveBeenCalledOnce();
    expect(port.events('transport')).toHaveLength(0);
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 1, state: 'stopped', hostTime: 0, mediaTime: 0, rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    driver.refresh();
    expect(session.route).toBe('pending'); // an untagged in-flight clock is not readiness
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 2, epoch: 1, state: 'stopped', hostTime: 0, mediaTime: 0, rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5, 0));
    driver.refresh();
    expect(session.route).toBe('engine');
    expect(session.frames).toBe(0);
    expect(port.events('transport').map(e => (e.message as any).body.command)).toEqual(['audioScrub', 'setLoop', 'seek', 'play']);
    expect(port.events('viewer').at(-1)?.message).toMatchObject({ body: { headless: true, composition: 'C' } });
    // Attaching presentation to the same session does not send another Play.
    session.sendViewerState({ composition: 'C', viewSeq: 2, headless: false } as never);
    port.deliver(envelope({ kind: 'event', body: { type: 'frame', revision: 3 } } as never, 5));
    driver.refresh();
    expect(port.events('transport').filter(e => (e.message as any).body.command === 'play')).toHaveLength(1);
    session.close(); connection.close('test complete');
  });

  it('a viewerless session follows the open composition before each command', async () => {
    const { connection, port } = await connected();
    const doc = stream(); doc.set(0);
    const session = new DocumentSession({ connection, stream: doc, documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    session.sendHeadless('C1');
    let open = 'C1';
    // install.ts's follower, on a bus like the kernel's.
    const listeners = new Map<string, Set<() => void>>();
    const bus = {
      on: (event: string, listener: () => void) => {
        if (!listeners.has(event)) listeners.set(event, new Set());
        listeners.get(event)!.add(listener);
        return () => listeners.get(event)!.delete(listener);
      },
      emit: (event: string) => { for (const listener of listeners.get(event) ?? []) listener(); }
    };
    const follower = followOpenComposition({ session: () => session, open: () => open, bus });
    expect([...listeners.keys()].sort()).toEqual(['comps', 'project']);
    const driver = new EngineTransport(() => session, new HostClock(async () => 0), { exclusive: true, wake: () => {}, prepare: follower.follow });
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 1, state: 'stopped', hostTime: 0, mediaTime: 0, rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5, 0));
    driver.refresh();
    expect(session.route).toBe('engine');
    // Another timeline tab: the same project, another composition (a
    // document batch, not a load). Play plays the composition just opened.
    open = 'C2';
    const start = port.sent.length;
    driver.play(0.25);
    const after = port.sent.slice(start).filter((e) => e.message.kind === 'event').map((e) => {
      const body = (e.message as { body: { type: string; command?: string; composition?: string } }).body;
      return body.type === 'viewer' ? `viewer:${body.composition}` : `${body.type}:${body.command}`;
    });
    expect(after[0]).toBe('viewer:C2');
    expect(after.slice(-2)).toEqual(['transport:seek', 'transport:play']);
    expect(session.composition).toBe('C2');
    // Nothing new to send for the same composition.
    const again = port.events('viewer').length;
    driver.pause();
    expect(port.events('viewer')).toHaveLength(again);
    // compositions.open (and its undo) announce `comps`; a project change `project`.
    open = 'C3';
    bus.emit('comps');
    expect(session.composition).toBe('C3');
    open = 'C1';
    bus.emit('project');
    expect(session.composition).toBe('C1');
    // A session with a viewer sends its own state: the follower leaves it.
    session.sendViewerState({ composition: 'C1', viewSeq: 9, headless: false } as never);
    open = 'C2';
    bus.emit('comps');
    expect(session.composition).toBe('C1');
    follower.dispose();
    expect([...listeners.values()].every((set) => set.size === 0)).toBe(true);
    session.close(); connection.close('test complete');
  });

  it('a viewerless session keeps the transport for a composition the engine cannot draw', async () => {
    const { connection, port } = await connected();
    const doc = stream(); doc.set(0);
    const session = new DocumentSession({ connection, stream: doc, documentId: () => 'P', log: () => {}, handlers: { message: () => {}, closed: () => {} } });
    await session.start();
    session.sendHeadless('C');
    const driver = new EngineTransport(() => session, new HostClock(async () => 0), { exclusive: true, wake: () => {} });
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 0, error: { code: 'unsupported', scope: { kind: 'session', session: 5 }, message: 'The new engine cannot draw this composition yet: path masks.', retry: { kind: 'never' }, detail: null } } } as never, 5, 0));
    // Its clock (the engine runs the transport) comes untagged and keeps the reason.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 1, epoch: 1, state: 'stopped', hostTime: 0, mediaTime: 0, rate: { num: 0, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    driver.refresh();
    expect(session.route).toBe('legacy');
    expect(session.routeReason).toMatch(/path masks/);
    expect(driver.active()).toBe(true);
    driver.play(0.5);
    expect(port.events('transport').map(e => (e.message as any).body.command)).toEqual(['audioScrub', 'seek', 'play']);
    // An edit or a composition switch makes it drawable: the engine's clock
    // comes tagged with the revision again, and the session is the engine's.
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 2, epoch: 2, state: 'playing', hostTime: 0, mediaTime: 0, rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5, 1));
    expect(session.route).toBe('engine');
    expect(session.routeReason).toBeNull();
    // And back: the next report routes it again.
    port.deliver(envelope({ kind: 'event', body: { type: 'failure', revision: 2, error: { code: 'unsupported', scope: { kind: 'session', session: 5 }, message: 'The new engine cannot draw this composition yet: path masks.', retry: { kind: 'never' }, detail: null } } } as never, 5, 2));
    port.deliver(envelope({ kind: 'event', body: { type: 'clock', seq: 3, epoch: 2, state: 'playing', hostTime: 0, mediaTime: 0, rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48000, loopRange: null } } as never, 5));
    expect(session.route).toBe('legacy');
    session.close(); connection.close('test complete');
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
