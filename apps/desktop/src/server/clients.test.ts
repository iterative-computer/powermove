import { describe, expect, it, vi } from 'vitest';

import { decodeFrame, encodeFrame, type ServerMessage } from '../shared/wire';
import { RemoteClient, WebIpcMain, type Socket } from './clients';

function harness() {
  const ipc = new WebIpcMain();
  const sent: ServerMessage[] = [];
  const socket: Socket = { send: (data) => { sent.push(decodeFrame(data) as ServerMessage); }, close: vi.fn() };
  const client = ipc.connect(socket, 'http://host/');
  const deliver = (message: unknown) => client.receive(encodeFrame(message));
  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
  return { ipc, client, sent, deliver, flush };
}

describe('WebIpcMain', () => {
  it('answers invoke through the registered handler with the client as sender', async () => {
    const { ipc, client, sent, deliver, flush } = harness();
    ipc.handle('app:ping', (event, value) => { expect(event.sender).toBe(client); expect(event.senderFrame?.url).toBe('http://host/'); return `pong:${String(value)}`; });
    deliver({ t: 'invoke', id: 3, ch: 'app:ping', args: ['x'] });
    await flush();
    expect(sent).toEqual([{ t: 'result', id: 3, ok: true, value: 'pong:x' }]);
  });

  it('reports handler failures and unknown channels as errors', async () => {
    const { ipc, sent, deliver, flush } = harness();
    ipc.handle('boom', () => { throw new Error('nope'); });
    deliver({ t: 'invoke', id: 1, ch: 'boom', args: [] });
    deliver({ t: 'invoke', id: 2, ch: 'missing', args: [] });
    await flush();
    expect(sent).toEqual([
      { t: 'result', id: 1, ok: false, error: 'nope' },
      { t: 'result', id: 2, ok: false, error: "No handler registered for 'missing'" }
    ]);
  });

  it('serves sendSync channels from the on-listener returnValue', async () => {
    const { ipc, sent, deliver, flush } = harness();
    ipc.on('store:get-sync', (event, payload) => { event.returnValue = `value-for-${(payload as { key: string }).key}`; });
    deliver({ t: 'sync', id: 9, ch: 'store:get-sync', args: [{ key: 'theme' }] });
    await flush();
    expect(sent).toEqual([{ t: 'result', id: 9, ok: true, value: 'value-for-theme' }]);
  });

  it('routes one-way sends to every listener and keeps binary payloads', async () => {
    const { ipc, deliver } = harness();
    const seen: unknown[] = [];
    ipc.on('log', (_event, payload) => seen.push(payload));
    ipc.on('log', (_event, payload) => seen.push(payload));
    deliver({ t: 'send', ch: 'log', args: [{ data: new Uint8Array([7, 8]) }] });
    expect(seen).toHaveLength(2);
    expect([...((seen[0] as { data: Uint8Array }).data)]).toEqual([7, 8]);
  });

  it('refuses a second handler for one channel', () => {
    const { ipc } = harness();
    ipc.handle('x', () => 1);
    expect(() => ipc.handle('x', () => 2)).toThrow(/second handler/);
  });
});

describe('RemoteClient', () => {
  it('pushes main → renderer events and stops after destroy', () => {
    const { client, sent } = harness();
    client.send('codex:event', { id: 'r', kind: 'progress', text: 'hi' });
    client.destroy();
    client.send('codex:event', { id: 'r', kind: 'progress', text: 'late' });
    expect(sent).toEqual([{ t: 'event', ch: 'codex:event', args: [{ id: 'r', kind: 'progress', text: 'hi' }] }]);
    expect(client.isDestroyed()).toBe(true);
  });

  it('asks the browser a question and resolves with its answer', async () => {
    const { client, sent, deliver } = harness();
    const answer = client.ask<number>('web:ask:message-box', { message: 'Save?' });
    expect(sent[0]).toEqual({ t: 'ask', id: 1, ch: 'web:ask:message-box', args: [{ message: 'Save?' }] });
    deliver({ t: 'answer', id: 1, value: 2 });
    await expect(answer).resolves.toBe(2);
  });

  it('rejects open questions and emits destroyed when the socket goes away', async () => {
    const { ipc, client } = harness();
    const destroyed = vi.fn();
    client.once('destroyed', destroyed);
    const answer = client.ask('web:ask:message-box', {});
    client.destroy();
    await expect(answer).rejects.toThrow(/closed/);
    expect(destroyed).toHaveBeenCalledOnce();
    expect(ipc.all()).toEqual([]);
    expect(ipc.fromWebContents(client)).toBeNull();
  });

  it('maps a client to its window and tracks the current one', () => {
    const ipc = new WebIpcMain();
    const socket: Socket = { send: vi.fn(), close: vi.fn() };
    const first = ipc.connect(socket, 'http://host/');
    const second = ipc.connect(socket, 'http://host/');
    expect(ipc.current()).toBe(second);
    expect(ipc.fromWebContents(first)).toBe(first.window);
    expect(first.window.webContents).toBe(first);
    expect(ipc.fromWebContents({})).toBeNull();
    second.destroy();
    expect(ipc.current()).toBe(first);
    expect(first).toBeInstanceOf(RemoteClient);
  });

  it('ignores malformed frames and messages', () => {
    const { client, sent } = harness();
    client.receive(new Uint8Array([0, 0]));
    client.receive(encodeFrame({ t: 'event', ch: 'x', args: [] }));
    expect(sent).toEqual([]);
  });
});

describe('reachableAddresses', () => {
  it('recognises the Tailscale CGNAT range', async () => {
    const { isTailnetAddress } = await import('./addresses');
    expect(isTailnetAddress('100.64.0.1')).toBe(true);
    expect(isTailnetAddress('100.127.255.9')).toBe(true);
    expect(isTailnetAddress('100.128.0.1')).toBe(false);
    expect(isTailnetAddress('192.168.1.4')).toBe(false);
  });

  it('prints only the bound host when it is explicit', async () => {
    const { reachableAddresses } = await import('./addresses');
    expect(reachableAddresses('127.0.0.1', 4747)).toEqual(['http://127.0.0.1:4747']);
    expect(reachableAddresses('0.0.0.0', 4747)[0]).toBe('http://localhost:4747');
  });
});
