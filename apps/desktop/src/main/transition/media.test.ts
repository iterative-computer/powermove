import { mkdtemp, readFile, readdir, rm, chmod, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TransitionMedia, registerTransition } from './media';
import { InProcessExecutor } from '../media-store/hasher';
import { blake3Hex } from '../../shared/blake3';
import type { TRPayload } from '../../shared/transition';
let root: string;
const executor = new InProcessExecutor();
const options = () => ({ root, executor, freeBytes: async () => 20 * 1024 ** 3 });
beforeEach(async () => { root = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
async function upload(service: TransitionMedia, key: string, data = new Uint8Array([1, 2, 3]), pass?: string, extra: Partial<TRPayload> = {}) {
  const payload = { key, size: data.length, type: 'video/webm', revision: 'r1', ...extra };
  const token = await service.request(7, { action: 'begin', payload, pass });
  if (data.length) await service.request(7, { action: 'chunk', token, offset: 0, data });
  const id = await service.request(7, { action: 'finish', token, hash: blake3Hex(data) });
  return { id, payload, hash: blake3Hex(data) };
}
async function record(service: TransitionMedia, pass: string, locator: string, ids: string[]) {
  await service.request(7, { action: 'record', pass, locator, metadata: { unknown: ['retained'] }, payloads: ids });
}
it('copies originals, posters, zero-byte blobs and unknown records; readiness waits for all handoffs', async () => {
  const service = new TransitionMedia({ ...options(), secrets: async () => {} });
  const inventory = ['assets:one', 'assets:poster', 'unknown:one'];
  const pass = await service.request(7, { action: 'start', inventory });
  const a = await upload(service, 'media:one', undefined, pass);
  const b = await upload(service, 'media:one:poster', new Uint8Array(), pass);
  await record(service, pass, inventory[0]!, [a.id]); await record(service, pass, inventory[1]!, [b.id]);
  await expect(service.request(7, { action: 'complete', pass, inventory })).rejects.toThrow('delta');
  await record(service, pass, inventory[2]!, []);
  expect(await service.request(7, { action: 'complete', pass, inventory })).toMatchObject({ mediaComplete: true, cutoverReady: false, done: 3 });
  await service.request(7, { action: 'localStorage', values: { unknown: { raw: 'exact' } } });
  expect(await service.request(7, { action: 'secrets' })).toMatchObject({ cutoverReady: true });
  expect(await readFile(service.store.objectPath(a.hash))).toEqual(Buffer.from([1, 2, 3]));
  expect((await readdir(path.join(root, 'keys'))).every(x => /^[a-f0-9]{64}\.json$/.test(x))).toBe(true);
});
it('resumes a killed pass by verifying and reusing receipts without another upload', async () => {
  let service = new TransitionMedia(options());
  let pass = await service.request(7, { action: 'start', inventory: ['one'] });
  const saved = await upload(service, 'media:one', undefined, pass);
  await service.release(7);
  service = new TransitionMedia(options());
  expect(await service.request(7, { action: 'status' })).toMatchObject({ mediaComplete: false });
  pass = await service.request(7, { action: 'start', inventory: ['one'] });
  const begin = vi.spyOn(service.store, 'beginStream');
  expect(await service.request(7, { action: 'reuse', pass, ...saved })).toBe(saved.id);
  expect(begin).not.toHaveBeenCalled();
  await record(service, pass, 'one', [saved.id]);
  expect(await service.request(7, { action: 'complete', pass, inventory: ['one'] })).toMatchObject({ mediaComplete: true });
});
it('retains both bytes and mappings on migration conflicts and blocks completion', async () => {
  const service = new TransitionMedia(options());
  const old = await upload(service, 'media:one', new Uint8Array([4]));
  const pass = await service.request(7, { action: 'start', inventory: ['one'] });
  const incoming = await upload(service, 'media:one', new Uint8Array([5]), pass);
  expect((await service.store.record('media:one'))?.hash).toBe(old.hash);
  expect(await service.store.hasObject(incoming.hash)).toBe(true);
  await expect(record(service, pass, 'one', [incoming.id])).rejects.toThrow('conflicts');
  expect(JSON.parse(await readFile(path.join(root, 'migration.json'), 'utf8')).payloads[incoming.id].conflict).toBe(true);
});
it('archives intentional import revisions and preserves prior localStorage snapshots', async () => {
  const service = new TransitionMedia(options());
  const first = await upload(service, 'media:one', new Uint8Array([4]));
  const second = await upload(service, 'media:one', new Uint8Array([5]));
  expect((await service.store.record('media:one'))?.hash).toBe(second.hash);
  expect(await service.store.hasObject(first.hash)).toBe(true);
  const history = await readdir(path.join(root, 'key-history'));
  const archived = JSON.parse(await readFile(path.join(root, 'key-history', history[0]!, (await readdir(path.join(root, 'key-history', history[0]!)))[0]!), 'utf8'));
  expect(archived.hash).toBe(first.hash);
  for (const raw of ['first', 'second']) await service.request(7, { action: 'localStorage', values: { old: { raw } } });
  expect(await readdir(path.join(root, 'local-storage'))).toHaveLength(2);
});
it('joins proxy provenance from project metadata rather than inferring an original from a media key', async () => {
  const proxyOf = { fingerprint: `v2:3:${'a'.repeat(64)}`, sourcePath: '/synthetic/original.mov' };
  const service = new TransitionMedia({ ...options(), references: async () => ({ 'media:proxy': [proxyOf] }) });
  const pass = await service.request(7, { action: 'start', inventory: ['one'] });
  const saved = await upload(service, 'media:proxy', undefined, pass);
  const value = JSON.parse(await readFile(path.join(root, 'migration.json'), 'utf8')).payloads[saved.id];
  expect(value).toMatchObject({ classification: 'proxy-of', proxyOf: [proxyOf] });
  expect((await service.store.record('media:proxy'))?.source).toBe('rendition');
});
it('rejects low space before staging and corrupt checkpoints without replacing them', async () => {
  const service = new TransitionMedia({ ...options(), freeBytes: async () => 1024 });
  await expect(upload(service, 'media:one')).rejects.toThrow('space');
  expect(await readdir(path.join(root, 'tmp'))).toEqual([]);
  await writeFile(path.join(root, 'migration.json'), '{damaged');
  const broken = new TransitionMedia(options());
  await expect(broken.initialize()).rejects.toThrow();
  expect(await readFile(path.join(root, 'migration.json'), 'utf8')).toBe('{damaged');
});
it('detects corrupted objects on resume and never claims verified completion', async () => {
  const service = new TransitionMedia(options());
  const pass = await service.request(7, { action: 'start', inventory: ['one'] });
  const saved = await upload(service, 'media:one', undefined, pass);
  await chmod(service.store.objectPath(saved.hash), 0o600);
  await writeFile(service.store.objectPath(saved.hash), new Uint8Array([7, 8, 9]));
  await expect(service.request(7, { action: 'reuse', pass, ...saved })).rejects.toThrow('damaged');
});
it('blocks new imports racing completion, changed inventories, foreign windows and incomplete bytes', async () => {
  const service = new TransitionMedia(options());
  const pass = await service.request(7, { action: 'start', inventory: [] });
  await expect(service.request(8, { action: 'complete', pass, inventory: [] })).rejects.toThrow('owned');
  await expect(service.request(7, { action: 'complete', pass, inventory: ['new'] })).rejects.toThrow('delta');
  await upload(service, 'new');
  await expect(service.request(7, { action: 'complete', pass, inventory: [] })).rejects.toThrow('imports');
  const token = await service.request(7, { action: 'begin', payload: { key: 'short', type: '', size: 3 } });
  await expect(service.request(7, { action: 'finish', token, hash: blake3Hex(new Uint8Array()) })).rejects.toThrow('incomplete');
});
it('rejects a sender outside the trusted app frame before touching storage', () => {
  const handle = vi.fn(); registerTransition({ handle } as any, new TransitionMedia(options()), () => false);
  expect(() => handle.mock.calls[0]![1]({ sender: { id: 3 } }, { action: 'status' })).toThrow('Untrusted');
});

it('releases crashed/navigated documents so another window can resume the same checkpoint', async () => {
  const { EventEmitter } = await import('node:events');
  const service = new TransitionMedia(options());
  const handle = vi.fn(); registerTransition({ handle } as any, service, () => true);
  const sender = Object.assign(new EventEmitter(), { id: 7 });
  await handle.mock.calls[0]![1]({ sender }, { action: 'start', inventory: ['one'] });
  sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
  const pass = await service.request(8, { action: 'start', inventory: [] });
  expect(await service.request(8, { action: 'complete', pass, inventory: [] })).toMatchObject({ mediaComplete: true });
});

it('replays a crash between a durable receipt and key publication without losing the copied object', async () => {
  let service = new TransitionMedia(options());
  let pass = await service.request(7, { action: 'start', inventory: ['one'] });
  vi.spyOn(service.store, 'publishRecord').mockRejectedValueOnce(new Error('synthetic publication interruption'));
  await expect(upload(service, 'media:one', undefined, pass)).rejects.toThrow('interruption');
  const checkpoint = JSON.parse(await readFile(path.join(root, 'migration.json'), 'utf8'));
  const receipt: any = Object.values(checkpoint.payloads)[0];
  expect(await service.store.hasObject(receipt.hash)).toBe(true);
  service = new TransitionMedia(options());
  pass = await service.request(7, { action: 'start', inventory: ['one'] });
  const saved = await upload(service, 'media:one', undefined, pass);
  await record(service, pass, 'one', [saved.id]);
  expect(await service.request(7, { action: 'complete', pass, inventory: ['one'] })).toMatchObject({ mediaComplete: true });
});
it('rejects a corrupt existing object before deduplicating another import onto it', async () => {
  const service = new TransitionMedia(options());
  const saved = await upload(service, 'media:one');
  await chmod(service.store.objectPath(saved.hash), 0o600);
  await writeFile(service.store.objectPath(saved.hash), new Uint8Array([8, 8, 8]));
  await expect(upload(service, 'media:two')).rejects.toThrow('damaged');
  expect(await service.store.record('media:two')).toBeNull();
});
it('blocks a pass started during an IDB import, including the time before CAS upload begins', async () => {
  const service = new TransitionMedia(options());
  const token = await service.request(7, { action: 'importing' });
  let pass = await service.request(8, { action: 'start', inventory: [] });
  await expect(service.request(8, { action: 'complete', pass, inventory: [] })).rejects.toThrow('delta');
  await expect(service.request(8, { action: 'imported', token })).rejects.toThrow('Unknown');
  await service.request(7, { action: 'imported', token });
  await expect(service.request(8, { action: 'complete', pass, inventory: [] })).rejects.toThrow('imports');
  pass = await service.request(8, { action: 'start', inventory: [] });
  expect(await service.request(8, { action: 'complete', pass, inventory: [] })).toMatchObject({ mediaComplete: true });
});
it('keeps completion blocked by a direct upload begun before a pass, and invalidates on finish', async () => {
  const service = new TransitionMedia(options());
  const data = new Uint8Array([1]);
  const token = await service.request(7, { action: 'begin', payload: { key: 'new', size: 1, type: '' } });
  const pass = await service.request(8, { action: 'start', inventory: [] });
  await expect(service.request(8, { action: 'complete', pass, inventory: [] })).rejects.toThrow('delta');
  await service.request(7, { action: 'chunk', token, offset: 0, data });
  await service.request(7, { action: 'finish', token, hash: blake3Hex(data) });
  await expect(service.request(8, { action: 'complete', pass, inventory: [] })).rejects.toThrow('imports');
  expect(await service.request(7, { action: 'status' })).toMatchObject({ mediaComplete: false });
});
it('releases abandoned import leases when the document dies without losing the next delta', async () => {
  const service = new TransitionMedia(options());
  await service.request(7, { action: 'importing' });
  await service.release(7);
  expect(await service.request(8, { action: 'status' })).toMatchObject({ mediaComplete: false });
  const pass = await service.request(8, { action: 'start', inventory: [] });
  expect(await service.request(8, { action: 'complete', pass, inventory: [] })).toMatchObject({ mediaComplete: true });
});
