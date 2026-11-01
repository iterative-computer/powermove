import { afterEach, expect, it, vi } from 'vitest';
import { exportLocalStorage, logical, prepareTransition, browserRecords, transfer, installTransitionNotice } from './transition';
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
const storage = (values: Record<string, string>) => ({ get length() { return Object.keys(values).length; }, key: (i: number) => Object.keys(values)[i] ?? null, getItem: (key: string) => values[key] ?? null }) as Storage;
it('exports all exact localStorage strings with parsed JSON and safe unknown keys', () => {
  const values = JSON.parse('{"unknown":"opaque","json":"{\\"value\\":3}","__proto__":"literal"}');
  const result = exportLocalStorage(storage(values));
  expect(result).toEqual({ unknown: { raw: 'opaque' }, json: { raw: '{"value":3}', json: { value: 3 } }, ['__proto__']: { raw: 'literal' } });
});
it('preserves unknown structured-clone values, cycles and nested Blob descriptors as logical JSON', () => {
  const value: any = { nested: new Map([['poster', new Blob(['poster'])]]), date: new Date('2020-01-01Z'), binary: new Uint8Array([1, 2]), unknown: undefined, number: Infinity, bigint: 2n };
  value.self = value;
  const result = logical(value);
  expect(result.blobs).toHaveLength(1);
  const json = JSON.stringify(result.metadata);
  for (const type of ['map', 'blob', '$ref', 'date', 'Uint8Array', 'undefined', 'bigint']) expect(json).toContain(type);
});
it('enumerates every object store and resolves only after its read transaction commits', async () => {
  vi.stubGlobal('location', { origin: 'app://powermove' });
  const rows: Record<string, any[]> = { assets: [{ key: 'media:one', value: { blob: new Blob(['bytes']) } }], unknown: [{ key: [3, new Date('2020-01-01Z')], value: { x: 2 } }] };
  let committed = false, closed = false;
  const db = { objectStoreNames: Object.keys(rows), close: () => { closed = true; }, transaction(names: string[]) {
    const tx: any = { objectStore(name: string) {
      const request: any = {}; let index = 0;
      const next = () => queueMicrotask(() => {
        const row = rows[name]![index++]; request.result = row ? { primaryKey: row.key, value: row.value, continue: next } : null;
        request.onsuccess?.();
      }); next(); return { openCursor: () => request };
    } };
    setTimeout(() => { committed = true; tx.oncomplete(); }, 0); return tx;
  } };
  const factory = { open() { const request: any = { result: db }; queueMicrotask(() => request.onsuccess()); return request; } };
  const records = await browserRecords(factory as unknown as IDBFactory);
  expect(records).toHaveLength(2); expect(committed && closed).toBe(true); expect(records.some(e => e.locator.includes('unknown'))).toBe(true);
});
it('refuses same-key Blob mutations during the final delta pass', async () => {
  const host = { request: vi.fn(async (command: any) => command.action === 'start' ? 'pass' : command.action === 'reuse' ? null : 'token') };
  let pass = 0;
  const records = async () => [{ locator: 'one', key: 'media:one', value: { blob: new Blob([++pass === 1 ? 'before' : 'after']) } }];
  await expect(prepareTransition(host, records, storage({}), () => {})).rejects.toThrow('changed');
  expect(host.request.mock.calls.some(([c]) => c.action === 'complete')).toBe(false);
});
it('aborts partial uploads after host errors and streams in bounded chunks', async () => {
  const calls: any[] = [];
  const host = { request: vi.fn(async (command: any) => { calls.push(command); if (command.action === 'chunk') throw new Error('disk full'); return 'token'; }) };
  await expect(transfer(host, { key: 'media:one', size: 2 * 1024 ** 2, type: '' }, new Blob([new Uint8Array(2 * 1024 ** 2)]))).rejects.toThrow('disk full');
  expect(calls.find(c => c.action === 'chunk').data.byteLength).toBe(1024 ** 2);
  expect(calls.at(-1).action).toBe('abort');
});

it('does not repeatedly hash a stable completed library during background polls', async () => {
  vi.useFakeTimers();
  vi.stubGlobal('window', { localStorage: storage({ setting: 'exact' }), indexedDB: { open: vi.fn() } });
  const host = { request: vi.fn(async (command: any) => command.action === 'status' ? { mediaComplete: true } : undefined) };
  const PM = { projectReady: Promise.resolve(), toast: vi.fn(), dismissToast: vi.fn() };
  const stop = installTransitionNotice(PM, host);
  try {
    await Promise.resolve(); await vi.advanceTimersByTimeAsync(5000); await vi.advanceTimersByTimeAsync(60_000);
    expect(host.request.mock.calls.filter(([command]) => command.action === 'status')).toHaveLength(2);
    expect(window.indexedDB.open).not.toHaveBeenCalled();
    expect(PM.toast).not.toHaveBeenCalled();
  } finally { stop(); }
});
