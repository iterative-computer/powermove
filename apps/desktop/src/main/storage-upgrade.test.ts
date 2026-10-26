import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { STORAGE_UPGRADE_FILE, installStorageUpgrade, StorageUpgrade, visitSourceReferences, type StorageUpgradeDeps } from './storage-upgrade';
import { STORAGE_IPC } from '../shared/storage-management';

let root: string;
beforeEach(async () => { root = await mkdtemp(path.join(tmpdir(), 'powermove-storage-upgrade-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
function fixture() {
  let bytes = 100_000;
  const deps: StorageUpgradeDeps = { root, measure: vi.fn(async () => bytes), exclusive: action => action(),
    recovery: vi.fn(async () => { bytes -= 20_000; }), sources: vi.fn(async () => { bytes -= 30_000; }), cleanup: vi.fn(async () => { bytes -= 10_000; }) };
  return { deps, upgrade: new StorageUpgrade(deps) };
}

it('runs once, checkpoints each stage, reports savings and remembers notification acknowledgment across restarts', async () => {
  const { deps, upgrade } = fixture(), owner = {}, progress = vi.fn();
  const start = await upgrade.request('app://powermove', owner, { action: 'start' }, progress);
  expect(start.state).toBe('browser');
  expect(deps.recovery).toHaveBeenCalledOnce(); expect(deps.sources).not.toHaveBeenCalled();
  const completed = await upgrade.request('app://powermove', owner, { action: 'browserComplete', token: start.token }, progress);
  expect(completed).toMatchObject({ state: 'complete', notify: true, freedBytes: 60_000 });
  expect(progress.mock.calls.map(([value]) => value.stage)).toEqual(['recovery', 'browser', 'browser', 'sources', 'cleanup']);
  const disk = JSON.parse(await readFile(path.join(root, STORAGE_UPGRADE_FILE), 'utf8'));
  expect(disk.origins['app://powermove'].steps).toEqual(['recovery', 'browser', 'sources', 'cleanup']);
  await upgrade.request('app://powermove', owner, { action: 'notified', token: completed.token });
  const reopened = new StorageUpgrade(deps);
  expect(await reopened.request('app://powermove', {}, { action: 'start' })).toEqual({ state: 'complete', notify: false });
  expect(deps.recovery).toHaveBeenCalledOnce(); expect(deps.sources).toHaveBeenCalledOnce(); expect(deps.cleanup).toHaveBeenCalledOnce();
});

it('resumes failed cleanup without repeating completed stages or losing a pending completion notice', async () => {
  const { deps, upgrade } = fixture(), owner = {};
  const first = await upgrade.request('app://powermove', owner, { action: 'start' });
  vi.mocked(deps.cleanup).mockRejectedValueOnce(new Error('AI is busy'));
  expect(await upgrade.request('app://powermove', owner, { action: 'browserComplete', token: first.token })).toEqual({ state: 'paused' });
  const restart = new StorageUpgrade(deps), nextOwner = {};
  const result = await restart.request('app://powermove', nextOwner, { action: 'start' });
  expect(result).toMatchObject({ state: 'complete', notify: true });
  expect(deps.recovery).toHaveBeenCalledOnce(); expect(deps.sources).toHaveBeenCalledOnce(); expect(deps.cleanup).toHaveBeenCalledTimes(2);
  restart.release(nextOwner);
  const afterCrash = new StorageUpgrade(deps);
  expect(await afterCrash.request('app://powermove', {}, { action: 'start' })).toMatchObject({ state: 'complete', notify: true });
  expect(deps.cleanup).toHaveBeenCalledTimes(2);
});

it('leases browser work to one document and rejects old tokens after an interrupted reload', async () => {
  const { upgrade } = fixture(), firstOwner = {}, secondOwner = {};
  const first = await upgrade.request('app://powermove', firstOwner, { action: 'start' });
  expect(await upgrade.request('app://powermove', secondOwner, { action: 'start' })).toEqual({ state: 'busy' });
  upgrade.release(firstOwner);
  const second = await upgrade.request('app://powermove', secondOwner, { action: 'start' });
  expect(second.token).not.toBe(first.token);
  await expect(upgrade.request('app://powermove', secondOwner, { action: 'browserComplete', token: first.token })).rejects.toThrow('Unknown');
  const complete = await upgrade.request('app://powermove', secondOwner, { action: 'browserComplete', token: second.token });
  expect(complete.state).toBe('complete');
});

it('does not checkpoint a running step when its renderer is lost, and repeats that step safely', async () => {
  const { deps, upgrade } = fixture(), owner = {};
  let release!: () => void;
  vi.mocked(deps.recovery).mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
  const running = upgrade.request('app://powermove', owner, { action: 'start' });
  await vi.waitFor(() => expect(deps.recovery).toHaveBeenCalledOnce());
  upgrade.release(owner); release();
  expect(await running).toEqual({ state: 'paused' });
  expect((await upgrade.request('app://powermove', {}, { action: 'start' })).state).toBe('browser');
  expect(deps.recovery).toHaveBeenCalledTimes(2);
});

it('tracks browser storage independently for packaged and development origins', async () => {
  const { deps, upgrade } = fixture(), owner = {};
  const first = await upgrade.request('app://powermove', owner, { action: 'start' });
  const complete = await upgrade.request('app://powermove', owner, { action: 'browserComplete', token: first.token });
  await upgrade.request('app://powermove', owner, { action: 'notified', token: complete.token });
  expect((await upgrade.request('http://localhost:5173', owner, { action: 'start' })).state).toBe('browser');
  expect(deps.recovery).toHaveBeenCalledTimes(2);
});

it('preserves an unreadable or malformed journal and retries instead of pretending cleanup completed', async () => {
  const file = path.join(root, STORAGE_UPGRADE_FILE); await writeFile(file, 'incomplete checkpoint');
  const { deps, upgrade } = fixture();
  expect(await upgrade.request('app://powermove', {}, { action: 'start' })).toEqual({ state: 'paused' });
  expect(deps.recovery).not.toHaveBeenCalled(); expect(await readFile(file, 'utf8')).toBe('incomplete checkpoint');
});

it('walks source references in snapshots and Undo without changing project content', async () => {
  const original = { assets: { clip: { id: 'clip', storageKey: 'media:clip', sourcePath: '/media/clip.mp4' } },
    history: { entries: [{ value: { id: 'deleted', sourcePath: '/media/deleted.png' } }] } };
  const before = structuredClone(original), visit = vi.fn(async () => {});
  await visitSourceReferences(original, visit);
  expect(visit).toHaveBeenCalledWith('media:clip', '/media/clip.mp4'); expect(visit).toHaveBeenCalledWith('deleted', '/media/deleted.png');
  expect(original).toEqual(before);
});

it('guards IPC and releases a browser claim on navigation', async () => {
  const { upgrade } = fixture(), handlers = new Map<string, Function>();
  installStorageUpgrade({ handle: (name, handler) => { handlers.set(name, handler); } }, upgrade, event => (event as any).trusted);
  const sender = Object.assign(new EventEmitter(), { getURL: () => 'app://powermove/index.html', isDestroyed: () => false, send: vi.fn() });
  const event = { sender, trusted: true };
  expect(() => handlers.get(STORAGE_IPC.upgrade)!({ ...event, trusted: false }, { action: 'start' })).toThrow('Unauthorized');
  const first = await handlers.get(STORAGE_IPC.upgrade)!(event, { action: 'start' });
  expect(first.state).toBe('browser');
  sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
  expect((await upgrade.request('app://powermove', {}, { action: 'start' })).state).toBe('browser');
});
