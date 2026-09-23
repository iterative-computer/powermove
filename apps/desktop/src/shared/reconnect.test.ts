import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Connection, type LinkSocket } from './link';
import { decodeFrame, encodeFrame } from './wire';
import { DISCONNECTED_MESSAGE, ReconnectingLink, isDisconnectError } from './reconnect';

/** A socket whose far end is the test: records frames, answers on demand, drops on command. */
class FakeSocket implements LinkSocket {
  binaryType = 'arraybuffer';
  readonly sent: Array<Record<string, unknown>> = [];
  private readonly handlers = new Map<string, Array<(event: { data: unknown }) => void>>();
  closedByClient = false;
  send(data: Uint8Array): void { this.sent.push(decodeFrame(data) as Record<string, unknown>); }
  close(): void { this.closedByClient = true; }
  addEventListener(type: string, listener: (event: { data: unknown }) => void): void {
    const list = this.handlers.get(type) ?? [];
    list.push(listener);
    this.handlers.set(type, list);
  }
  reply(id: number, value: unknown): void { this.deliver({ t: 'result', id, ok: true, value }); }
  event(ch: string, ...args: unknown[]): void { this.deliver({ t: 'event', ch, args }); }
  drop(): void { for (const handler of this.handlers.get('close') ?? []) handler({ data: undefined }); }
  private deliver(message: unknown): void { for (const handler of this.handlers.get('message') ?? []) handler({ data: encodeFrame(message as never) }); }
}

describe('ReconnectingLink', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  function setup(opts: { fail?: number } = {}) {
    const sockets: FakeSocket[] = [];
    let failures = opts.fail ?? 0;
    const link = new ReconnectingLink({
      open: async () => {
        if (failures > 0) { failures -= 1; throw new Error('unreachable'); }
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
      minDelayMs: 100,
      maxDelayMs: 1000
    });
    const first = new FakeSocket();
    sockets.push(first);
    link.adopt(new Connection(first));
    return { link, sockets };
  }

  it('holds calls made while down and sends them on the next socket, in order', async () => {
    const { link, sockets } = setup();
    sockets[0]!.drop();
    expect(link.isConnected).toBe(false);
    const pending = link.invoke('a', 1);
    link.send('b', 2);
    const later = link.invoke('c', 3);
    await vi.advanceTimersByTimeAsync(0);
    const next = sockets[1]!;
    expect(next).toBeDefined();
    expect(next.sent.map((frame) => frame['ch'])).toEqual(['a', 'b', 'c']);
    next.reply(next.sent[0]!['id'] as number, 'A');
    next.reply(next.sent[2]!['id'] as number, 'C');
    await expect(pending).resolves.toBe('A');
    await expect(later).resolves.toBe('C');
    expect(link.isConnected).toBe(true);
  });

  it('rejects in-flight calls when the socket drops, with an error callers can recognise', async () => {
    const { link, sockets } = setup();
    const inflight = link.invoke('x');
    sockets[0]!.drop();
    await expect(inflight).rejects.toThrow(/host closed/);
    await inflight.catch((error) => expect(isDisconnectError(error)).toBe(true));
  });

  it('backs off between failed attempts and announces the reconnect once', async () => {
    const { link, sockets } = setup({ fail: 2 });
    const reconnected = vi.fn();
    const closed = vi.fn();
    link.on('__reconnected', reconnected);
    link.on('__closed', closed);
    sockets[0]!.drop();
    expect(closed).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(0);     // attempt 1 fails
    expect(sockets).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(100);   // attempt 2 fails
    expect(sockets).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(200);   // attempt 3 succeeds
    expect(sockets).toHaveLength(2);
    expect(reconnected).toHaveBeenCalledTimes(1);
    expect(link.generation).toBe(2);
  });

  it('keeps listeners and answer handlers across sockets', async () => {
    const { link, sockets } = setup();
    const seen: unknown[] = [];
    link.on('evt', (value) => seen.push(value));
    link.answer('ask:thing', () => 42);
    sockets[0]!.event('evt', 'before');
    sockets[0]!.drop();
    await vi.advanceTimersByTimeAsync(0);
    const next = sockets[1]!;
    next.event('evt', 'after');
    expect(seen).toEqual(['before', 'after']);
    (next as unknown as { deliver(message: unknown): void }).deliver({ t: 'ask', id: 7, ch: 'ask:thing', args: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(next.sent.find((frame) => frame['t'] === 'answer')).toMatchObject({ id: 7, value: 42 });
  });

  it('gives up on a queued call after the wait limit', async () => {
    const sockets: FakeSocket[] = [];
    const link = new ReconnectingLink({ open: async () => { throw new Error('down'); }, waitMs: 1000, minDelayMs: 100, maxDelayMs: 100 });
    const first = new FakeSocket();
    sockets.push(first);
    link.adopt(new Connection(first));
    first.drop();
    const call = link.invoke('slow');
    const outcome = call.then(() => 'ok', (error: Error) => error.message);
    await vi.advanceTimersByTimeAsync(1100);
    expect(await outcome).toBe(DISCONNECTED_MESSAGE);
  });

  it('stops retrying once closed', async () => {
    const { link, sockets } = setup();
    sockets[0]!.drop();
    link.close();
    await vi.advanceTimersByTimeAsync(5000);
    expect(sockets).toHaveLength(1);
    await expect(link.invoke('x')).rejects.toThrow(/host closed/);
  });
});
