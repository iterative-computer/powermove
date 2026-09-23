import { afterEach, expect, it } from 'vitest';
import { MessageChannel } from 'node:worker_threads';
import { createRpc, rpcTransfers, SandboxTimeoutError } from './sandbox-rpc';
const close: Array<() => void> = [];
afterEach(() => { for (const fn of close.splice(0)) fn(); });
function pair(a: Record<string, (...args: any[]) => unknown> = {}, b: Record<string, (...args: any[]) => unknown> = {}, timeout = 100) {
  const channel = new MessageChannel();
  const left = createRpc(channel.port1 as unknown as MessagePort, a, timeout);
  const right = createRpc(channel.port2 as unknown as MessagePort, b, timeout);
  close.push(() => { left.close(); right.close(); });
  return { left, right };
}
it('round-trips calls and notifications', async () => {
  let value = 0;
  const { left } = pair({}, { sum: (a: number, b: number) => a + b, save: (n: number) => { value = n; } });
  expect(await left.call('sum', 3, 4)).toBe(7);
  left.notify('save', 9);
  await new Promise(resolve => setTimeout(resolve, 5));
  expect(value).toBe(9);
});
it('invokes and releases function handles', async () => {
  const { left, right } = pair();
  const id = right.handle((value: string) => value.toUpperCase());
  expect(await left.invokeHandle(id, 'hi')).toBe('HI');
  right.release(id);
  await expect(left.invokeHandle(id, 'hi')).rejects.toThrow('Unknown sandbox method');
});
it('preserves error names and codes', async () => {
  const { left } = pair({}, { fail() { const error = new Error('denied'); error.name = 'PermissionError'; (error as Error & { code: string }).code = 'full-access'; throw error; } });
  await expect(left.call('fail')).rejects.toMatchObject({ name: 'PermissionError', message: 'denied', code: 'full-access' });
});
it('can transfer an ArrayBuffer with a call', async () => {
  const { left } = pair({}, { bytes: (buffer: ArrayBuffer) => [...new Uint8Array(buffer)] });
  const buffer = Uint8Array.from([3, 7]).buffer;
  expect(await left.call('bytes', buffer, rpcTransfers(buffer))).toEqual([3, 7]);
  expect(buffer.byteLength).toBe(0);
});
it('times out and rejects outstanding work on close', async () => {
  const { left } = pair({}, { wait: () => new Promise(() => {}) }, 15);
  await expect(left.call('wait')).rejects.toBeInstanceOf(SandboxTimeoutError);
  const pending = left.call('wait');
  left.close();
  await expect(pending).rejects.toThrow('closed');
});
