// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PROTOCOL_BUILD, type Envelope } from '../../../shared/engine-protocol';
import { ENGINE_WINDOW_MESSAGE } from '../../../shared/engine-ipc';
import { decodeEnvelope, encodeEnvelope, envelope, localRanges } from '../../../main/engine/protocol';
import type { PortLike } from './connection';
import { ENGINE_VIEWER_SERVICE, installEngineConnection } from './install';

const harness = vi.hoisted(() => ({ renders: [] as any[] }));
vi.mock('../../../extensions/viewer/engine-present/prewarm', () => ({ PresentationWarmup: class { warm() {} dispose() {} } }));
vi.mock('./audio', () => ({ EngineAudio: class { sync() {} receive() {} }, routeAudioPanels: () => () => {} }));
vi.mock('./transport', () => ({ EngineTransport: class { refresh() {} resetLoop() {} seek() {} active() { return false; } now() { return null; } }, ticks: (value: number) => value }));
vi.mock('./app-captures', () => ({ installAppCaptures: () => () => {}, usesBlender: () => false }));
vi.mock('./render-transport', () => ({
  EngineSnapshotError: class extends Error { code = 'internal'; },
  EngineRenderTransport: class {
    constructor(options: any) { harness.renders.push(options); }
    provideCompat() { return () => {}; }
    async syncProducers() {}
    dispose() {}
  }
}));

class Port implements PortLike {
  sent: Envelope[] = [];
  closed = false;
  private messages: Array<(event: { data: unknown }) => void> = [];
  private closes: Array<() => void> = [];
  addEventListener(type: 'message' | 'close', listener: any) { (type === 'message' ? this.messages : this.closes).push(listener); }
  start() {}
  close() { if (!this.closed) { this.closed = true; for (const listener of this.closes) listener(); } }
  private deliver(message: Envelope) { if (!this.closed) for (const listener of this.messages) listener({ data: encodeEnvelope(message) }); }
  postMessage(bytes: unknown) {
    const message = decodeEnvelope(bytes as Uint8Array);
    this.sent.push(message);
    const body = message.message;
    if (body.kind === 'hello') queueMicrotask(() => this.deliver(envelope({ kind: 'welcome', role: 'interactiveEngine', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges().map(range => ({ contract: range.contract, version: 0 })), maxMessageBytes: 16 * 1024 * 1024 } as never)));
    else if (body.kind === 'request') {
      const request = body.body as any;
      const answer = request.type === 'openSession' ? { type: 'sessionOpened', session: 5 } : request.type === 'resync' ? { type: 'patchesApplied', document: request.document, revision: request.revision } : { type: 'ack' };
      queueMicrotask(() => this.deliver(envelope({ kind: 'response', id: body.id, body: answer } as never, request.type === 'openSession' ? null : 5, request.revision ?? null)));
    }
  }
  requests(type: string) { return this.sent.filter(item => item.message.kind === 'request' && (item.message.body as any).type === type); }
}

function deferred() {
  let resolve!: () => void, reject!: (reason: Error) => void;
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const disposals: Array<() => void> = [];
afterEach(() => { for (const dispose of disposals.splice(0)) dispose(); harness.renders.length = 0; vi.restoreAllMocks(); });

function setup(reconnect: (generation: number) => Promise<void>) {
  let service: any;
  const snapshot = vi.fn(() => { throw new Error('native reconnect must not serialize a snapshot'); });
  const doc = { enabled: true, ready: true, whenReady: Promise.resolve(true), reconnect,
    subscribe: () => () => {}, onPending: () => () => {}, flushBeforeListeners: () => () => {},
    resume: () => ({ revision: 3, hash: 'ab'.repeat(32), batches: [] }), snapshot };
  const PM: any = { proj: { id: 'P', compId: 'C', w: 320, h: 180 }, time: 0,
    EngineDoc: doc, bus: { on: () => () => {}, emit: vi.fn() }, assets: { map: new Map() },
    Kernel: { services: { register: (key: string, value: any) => { if (key === ENGINE_VIEWER_SERVICE) service = value; return { dispose() {} }; } } } };
  const native: any = { enabledSync: () => true, hostTime: async () => 0, requestPorts: vi.fn(), stageDocument: vi.fn() };
  const handle = installEngineConnection(PM, { nativeEngine: native, frameBridge: { onSurface: () => () => {}, videoFrame: () => null } as any }, window)!;
  disposals.push(() => handle.dispose());
  const ready = vi.fn(); service.onReady(ready);
  const port = (generation: number) => {
    const port = new Port();
    window.dispatchEvent(new MessageEvent('message', { source: window, data: { type: ENGINE_WINDOW_MESSAGE.port, process: 'interactive', generation }, ports: [port as unknown as MessagePort] }));
    return port;
  };
  const handlers = { message: vi.fn(), closed: vi.fn() };
  return { handle, native, service, ready, port, handlers, snapshot };
}

describe('interactive connection document recovery', () => {
  it('awaits generation recovery before ready notifications or opening a session', async () => {
    const recovery = deferred();
    const reconnect = vi.fn(() => recovery.promise);
    const test = setup(reconnect), port = test.port(2);
    await vi.waitFor(() => expect(reconnect).toHaveBeenCalledWith(2));
    expect(test.ready).not.toHaveBeenCalled();
    expect(test.handle.status().connected).toBe(false);
    expect(test.service.open(test.handlers)).toBeNull();
    expect(port.requests('openSession')).toHaveLength(0);
    expect(harness.renders[0].windowDocument()).toBeNull();
    const waking = harness.renders[0].wake();
    await Promise.resolve();
    expect(port.requests('openSession')).toHaveLength(0);
    recovery.resolve();
    await waking;
    expect(test.ready).toHaveBeenCalledTimes(1);
    expect(test.handle.status().connected).toBe(true);
    expect(port.requests('openSession')).toHaveLength(1);
    expect(port.requests('resync')).toHaveLength(1);
    expect(test.snapshot).not.toHaveBeenCalled();
  });

  it('gates a new generation after an existing viewer session was live', async () => {
    const recovery = deferred();
    const reconnect = vi.fn((generation: number) => generation === 1 ? Promise.resolve() : recovery.promise);
    const test = setup(reconnect), first = test.port(1);
    await vi.waitFor(() => expect(test.ready).toHaveBeenCalledTimes(1));
    expect(test.service.open(test.handlers)).not.toBeNull();
    await vi.waitFor(() => expect(first.requests('resync')).toHaveLength(1));
    const second = test.port(2);
    await vi.waitFor(() => expect(reconnect).toHaveBeenCalledWith(2));
    expect(first.closed).toBe(true);
    expect(test.handle.session()).toBeNull();
    expect(test.service.open(test.handlers)).toBeNull();
    expect(test.ready).toHaveBeenCalledTimes(1);
    recovery.resolve();
    await vi.waitFor(() => expect(test.ready).toHaveBeenCalledTimes(2));
    expect(test.service.open(test.handlers)).not.toBeNull();
    await vi.waitFor(() => expect(second.requests('resync')).toHaveLength(1));
    expect(test.snapshot).not.toHaveBeenCalled();
  });

  it('ignores stale recovery completion while a newer port is still recovering', async () => {
    const firstRecovery = deferred(), secondRecovery = deferred();
    const reconnect = vi.fn((generation: number) => generation === 1 ? firstRecovery.promise : secondRecovery.promise);
    const test = setup(reconnect), first = test.port(1);
    await vi.waitFor(() => expect(reconnect).toHaveBeenCalledWith(1));
    const second = test.port(2);
    await vi.waitFor(() => expect(reconnect).toHaveBeenCalledWith(2));
    firstRecovery.resolve();
    await Promise.resolve(); await Promise.resolve();
    expect(first.closed).toBe(true);
    expect(test.ready).not.toHaveBeenCalled();
    expect(test.handle.status().connected).toBe(false);
    expect(test.service.open(test.handlers)).toBeNull();
    secondRecovery.resolve();
    await vi.waitFor(() => expect(test.ready).toHaveBeenCalledTimes(1));
    expect(test.handle.status().generation).toBe(2);
    expect(test.service.open(test.handlers)).not.toBeNull();
    await vi.waitFor(() => expect(second.requests('resync')).toHaveLength(1));
  });

  it('does not close a recovered newer connection when an old recovery rejects', async () => {
    const old = deferred();
    const reconnect = vi.fn((generation: number) => generation === 1 ? old.promise : Promise.resolve());
    const test = setup(reconnect);
    test.port(1);
    await vi.waitFor(() => expect(reconnect).toHaveBeenCalledWith(1));
    const newest = test.port(2);
    await vi.waitFor(() => expect(test.ready).toHaveBeenCalledTimes(1));
    old.reject(new Error('the retired engine stopped'));
    await Promise.resolve(); await Promise.resolve();
    expect(newest.closed).toBe(false);
    expect(test.handle.status().connected).toBe(true);
    expect(test.ready).toHaveBeenCalledTimes(1);
    expect(test.native.requestPorts).toHaveBeenCalledTimes(1);
  });

  it('closes a failed recovery and uses the existing port retry backoff', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const recovery = deferred(), test = setup(() => recovery.promise), port = test.port(2);
    await vi.waitFor(() => expect(test.handle.status().generation).toBe(2));
    recovery.reject(new Error('native recovery failed'));
    await vi.waitFor(() => expect(port.closed).toBe(true));
    expect(test.ready).not.toHaveBeenCalled();
    expect(test.service.open(test.handlers)).toBeNull();
    expect(test.handle.status().connected).toBe(false);
    await vi.waitFor(() => expect(test.native.requestPorts).toHaveBeenCalledTimes(2));
  });
});
