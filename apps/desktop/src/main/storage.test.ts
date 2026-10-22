import { EventEmitter } from 'node:events';
import * as fs from 'node:fs/promises';
import * as syncFs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import type { App, IpcMain } from 'electron';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { IpcValidationError } from '../shared/guards';
import { IPC, LIMITS } from '../shared/ipc';
import {
  createStore,
  installQuitFlush,
  registerStoreIpc,
  type Store,
  type StoreChannel,
  type StoreFileSystem
} from './storage';
import type { StoreReply } from './store-engine';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-store-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(temporaryDirectories.splice(0).map((directory) => fs.rm(directory, { recursive: true })));
});

describe('file store', () => {
  it('indexes heavy filenames without reading them, and reads through every synchronous API', async () => {
    const directory = await temporaryDirectory();
    const prefixes = ['project', 'projectHistory', 'projectState', 'projectJournal', 'agentThreads', 'agentAttachment'];
    for (const prefix of prefixes) await fs.writeFile(path.join(directory, `${prefix}.saved.json`), '{"saved":true}');
    await fs.writeFile(path.join(directory, 'theme.json'), '"dark"');
    const readFile = vi.fn(fs.readFile), readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { fs: fileSystemWith({ readFile: readFile as typeof fs.readFile }), syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load();
    expect(readFile).toHaveBeenCalledTimes(1);
    expect(readFileSync).not.toHaveBeenCalled();
    const snapshot = store.snapshot();
    expect(snapshot).toEqual({ theme: 'dark', __powermoveLazyKeys: prefixes.map(prefix => `${prefix}.saved`).sort() });
    expect(store.snapshotSerialized!()).toEqual(store.bootstrapSerialized!());
    expect(store.get!('project.saved')).toEqual({ saved: true });
    expect(store.getSerialized!('projectHistory.saved')).toBe('{"saved":true}');
    expect(store.getEncodedSerialized!('agentAttachment.saved')).toBe('{"saved":true}');
    store.get!('project.saved');
    expect(readFileSync).toHaveBeenCalledTimes(3);
    expect(store.snapshot()).toEqual(snapshot);
  });

  it('deduplicates asynchronous preload and avoids synchronous reads, including oversized values', async () => {
    const directory = await temporaryDirectory();
    await fs.writeFile(path.join(directory, 'project.saved.json'), '{"saved":true}');
    const readFile = vi.fn(fs.readFile), readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { fs: fileSystemWith({ readFile: readFile as typeof fs.readFile }), syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load();
    await Promise.all([store.preload!('project.saved'), store.preloadProject!('saved')]);
    expect(store.get!('project.saved')).toEqual({ saved: true });
    expect(readFile).toHaveBeenCalledTimes(1);
    expect(readFileSync).not.toHaveBeenCalled();
    const uncached = createStore(directory, { heavyCacheBytes: 0, syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await uncached.load();
    expect(await uncached.getEncodedSerializedAsync!('project.saved')).toBe('{"saved":true}');
    expect(readFileSync).not.toHaveBeenCalled();
  });

  it.each(['sync', 'async'])('retains an oversized most-recent value across every read API after %s loading', async mode => {
    const directory = await temporaryDirectory();
    const serialized = JSON.stringify({ text: 'large'.repeat(100) });
    await fs.writeFile(path.join(directory, 'project.large.json'), serialized);
    const readFile = vi.fn(fs.readFile), readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { heavyCacheBytes: 16,
      fs: fileSystemWith({ readFile: readFile as typeof fs.readFile }),
      syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load();
    if (mode === 'async') await store.preloadProject!('large');
    else expect(store.get!('project.large')).toEqual(JSON.parse(serialized));
    for (let i = 0; i < 3; i++) {
      expect(store.get!('project.large')).toEqual(JSON.parse(serialized));
      expect(store.getSerialized!('project.large')).toBe(serialized);
      expect(store.getEncodedSerialized!('project.large')).toBe(serialized);
      expect(await store.getEncodedSerializedAsync!('project.large')).toBe(serialized);
    }
    expect(readFile).toHaveBeenCalledTimes(mode === 'async' ? 1 : 0);
    expect(readFileSync).toHaveBeenCalledTimes(mode === 'sync' ? 1 : 0);
  });

  it('evicts the least recently read clean values and reads them back from disk', async () => {
    const directory = await temporaryDirectory();
    for (const id of ['a', 'b', 'c']) await fs.writeFile(path.join(directory, `project.${id}.json`), '1');
    const readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { heavyCacheBytes: 4, syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load();
    for (const key of ['a', 'b', 'a', 'c', 'a']) expect(store.get!(`project.${key}`)).toBe(1);
    expect(readFileSync).toHaveBeenCalledTimes(3);
    expect(store.get!('project.b')).toBe(1);
    expect(readFileSync).toHaveBeenCalledTimes(4);
  });

  it.each([
    ['openWindows', ['large']],
    ['windowTabs', [{ tabs: ['large'], active: 'other' }]],
    ['openTabs', ['large']],
    ['lastActiveProject', 'large'],
  ])('pins oversized open projects from %s, excludes them from the budget, and releases them on close', async (key, metadata) => {
    const directory = await temporaryDirectory();
    const large = { text: 'large'.repeat(100) };
    await fs.writeFile(path.join(directory, `${key}.json`), JSON.stringify(metadata));
    for (const prefix of ['project', 'projectHistory', 'projectState', 'projectJournal', 'agentThreads']) {
      await fs.writeFile(path.join(directory, `${prefix}.large.json`), JSON.stringify(large));
    }
    for (const id of ['a', 'b', 'c']) await fs.writeFile(path.join(directory, `project.${id}.json`), '1');
    const readFile = vi.fn(fs.readFile), readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { heavyCacheBytes: 4,
      fs: fileSystemWith({ readFile: readFile as typeof fs.readFile }),
      syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load(); readFile.mockClear();
    await store.preloadProject!('large');
    // The two unpinned small values still fit: pinned bytes do not count.
    store.get!('project.a'); store.get!('project.b'); store.get!('project.a');
    expect(readFileSync).toHaveBeenCalledTimes(2);
    for (let i = 0; i < 3; i++) {
      for (const prefix of ['project', 'projectHistory', 'projectState', 'projectJournal', 'agentThreads']) {
        expect(store.get!(`${prefix}.large`)).toEqual(large);
      }
      store.get!('project.b');
    }
    expect(readFile).toHaveBeenCalledTimes(5);
    expect(readFileSync).toHaveBeenCalledTimes(2);
    store.set(key as string, null);
    store.get!('project.c');
    expect(store.get!('project.large')).toEqual(large);
    expect(readFileSync).toHaveBeenCalledTimes(4);
    await store.flushAll();
  });

  it('keeps project and history MRUs warm together without a session signal', async () => {
    const directory = await temporaryDirectory();
    for (const prefix of ['project', 'projectHistory']) await fs.writeFile(path.join(directory, `${prefix}.large.json`), '{"large":true}');
    const readFile = vi.fn(fs.readFile), readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { heavyCacheBytes: 0,
      fs: fileSystemWith({ readFile: readFile as typeof fs.readFile }),
      syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load(); await store.preloadProject!('large');
    for (let i = 0; i < 3; i++) {
      expect(store.get!('project.large')).toEqual({ large: true });
      expect(store.get!('projectHistory.large')).toEqual({ large: true });
    }
    expect(readFile).toHaveBeenCalledTimes(2);
    expect(readFileSync).not.toHaveBeenCalled();
  });

  it('holds independent session leases and releases once, without resurrecting deletes', async () => {
    const directory = await temporaryDirectory();
    for (const id of ['held', 'other']) await fs.writeFile(path.join(directory, `project.${id}.json`), '1');
    const readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { heavyCacheBytes: 0,
      syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await store.load();
    const first = store.acquireProject!('held'), second = store.acquireProject!('held');
    store.get!('project.held'); store.get!('project.other');
    first(); first();
    expect(store.get!('project.held')).toBe(1);
    store.get!('project.other'); second();
    expect(store.get!('project.held')).toBe(1);
    expect(readFileSync).toHaveBeenCalledTimes(3);
    const release = store.acquireProject!('held');
    store.delete('project.held'); release();
    expect(store.get!('project.held')).toBeUndefined();
    await store.flushAll();
  });

  it('pins attachment payloads referenced by an open project archive', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory, { heavyCacheBytes: 0 });
    await store.load();
    store.set('openWindows', ['active']);
    store.set('agentThreads.active', { threads: [{ attachments: [{ id: 'draft', payloadStored: true }],
      conversation: [{ attachments: [{ id: 'sent', payloadStored: true }] }] }] });
    for (const id of ['draft', 'sent', 'other']) store.set(`agentAttachment.${id}`, { content: id });
    await store.flushAll();
    const reader = vi.fn(syncFs.readFileSync);
    // Inspect disk accesses on reopening, after the archive identifies owners.
    const reopened = createStore(directory, { heavyCacheBytes: 0,
      syncFs: { ...syncFs, readFileSync: reader as typeof syncFs.readFileSync } });
    await reopened.load(); await reopened.preloadProject!('active');
    for (let i = 0; i < 3; i++) for (const id of ['draft', 'sent', 'other']) {
      expect(reopened.get!(`agentAttachment.${id}`)).toEqual({ content: id });
    }
    expect(reader).toHaveBeenCalledTimes(3);
    reopened.delete('openWindows');
    reopened.get!('agentAttachment.other');
    expect(reopened.get!('agentAttachment.draft')).toEqual({ content: 'draft' });
    expect(reader).toHaveBeenCalledTimes(4);
    await reopened.flushAll();
  });

  it('protects debounced and in-flight writes under cache pressure until they reach disk', async () => {
    const directory = await temporaryDirectory();
    let release!: () => void, started!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const writing = new Promise<void>(resolve => { started = resolve; });
    const writeFile: typeof fs.writeFile = async (...args) => { started(); await gate; return fs.writeFile(...args); };
    const readFileSync = vi.fn(syncFs.readFileSync);
    const store = createStore(directory, { heavyCacheBytes: 0, writeDelayMs: 60_000,
      fs: fileSystemWith({ writeFile }), syncFs: { ...syncFs, readFileSync: readFileSync as typeof syncFs.readFileSync } });
    await fs.writeFile(path.join(directory, 'project.pressure.json'), '{"clean":true}');
    await store.load();
    const releasePressure = store.acquireProject!('pressure');
    await store.preload!('project.pressure');
    store.set('project.dirty', { revision: 1 });
    store.get!('project.pressure');
    expect(store.get!('project.dirty')).toEqual({ revision: 1 });
    const flush = store.flushAll(); await writing;
    store.get!('project.pressure');
    expect(store.get!('project.dirty')).toEqual({ revision: 1 });
    store.set('project.dirty', { revision: 2 });
    store.get!('project.pressure');
    expect(store.get!('project.dirty')).toEqual({ revision: 2 });
    expect(readFileSync).not.toHaveBeenCalled();
    store.get!('project.pressure');
    release(); await flush; await store.flushAll();
    expect(store.get!('project.dirty')).toEqual({ revision: 2 });
    expect(readFileSync).toHaveBeenCalledTimes(1);
    expect(JSON.parse(await fs.readFile(path.join(directory, 'project.dirty.json'), 'utf8'))).toEqual({ revision: 2 });
    releasePressure();
  });

  it('keeps the only current copy cached after a failed write', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory, { heavyCacheBytes: 0, fs: fileSystemWith({ writeFile: async () => { throw new Error('disk full'); } }) });
    await store.load(); store.set('project.dirty', { recovered: true }); await store.flushAll();
    store.set('project.pressure', { other: true }); await store.flushAll();
    expect(store.get!('project.dirty')).toEqual({ recovered: true });
  });

  it.each(['set', 'delete'] as const)('does not resurrect an old async disk read after %s', async operation => {
    const directory = await temporaryDirectory(), file = path.join(directory, 'project.race.json');
    await fs.writeFile(file, '{"old":true}');
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const readFile = (async (...args: Parameters<typeof fs.readFile>) => { const result = await fs.readFile(...args); await gate; return result; }) as typeof fs.readFile;
    const store = createStore(directory, { fs: fileSystemWith({ readFile: readFile as typeof fs.readFile }), writeDelayMs: 60_000 });
    await store.load(); const reading = store.preload!('project.race');
    if (operation === 'set') store.set('project.race', { fresh: true }); else store.delete('project.race');
    release(); await reading;
    expect(store.get!('project.race')).toEqual(operation === 'set' ? { fresh: true } : undefined);
    await store.flushAll();
    expect(store.get!('project.race')).toEqual(operation === 'set' ? { fresh: true } : undefined);
    if (operation === 'delete') expect(store.snapshot().__powermoveLazyKeys).toBeUndefined();
  });

  it.each(['sync', 'async'])('quarantines corrupt heavy values on first %s access, not startup', async mode => {
    const directory = await temporaryDirectory(), file = path.join(directory, 'projectHistory.broken.json');
    await fs.writeFile(file, '{"truncated":');
    const store = createStore(directory, { now: () => 1234 }), errors: any[] = [];
    store.onError(error => errors.push(error)); await store.load();
    expect(errors).toEqual([]); expect(await fs.readFile(file, 'utf8')).toBe('{"truncated":');
    if (mode === 'async') await store.preload!('projectHistory.broken'); else store.get!('projectHistory.broken');
    expect(store.get!('projectHistory.broken')).toBeUndefined();
    expect(errors).toHaveLength(1); expect(errors[0].error).toContain('quarantined');
    expect(await fs.readFile(`${file}.corrupt-1234`, 'utf8')).toBe('{"truncated":');
    expect(store.snapshot().__powermoveLazyKeys).toBeUndefined();
  });

  it('preserves history sharing and contents across independent load and eviction', async () => {
    const directory = await temporaryDirectory();
    const history = { version: 1, index: 0, entries: [{ label: 'Edit',
      forward: [{ path: ['layers'], exists: true, value: [{ id: 'shared', text: 'same' }] }], backward: [] }] };
    const writer = createStore(directory); await writer.load();
    writer.set('projectHistory.a', history); writer.set('projectState.b', { history }); await writer.flushAll();
    const shared = createStore(directory); await shared.load();
    await shared.preload!('projectHistory.a'); await shared.preload!('projectState.b');
    // Main holds each key's encoded bytes (the engine packs histories), not
    // parsed objects, so there is no record identity to share: check contents.
    expect(shared.get!('projectHistory.a')).toEqual(history);
    expect(shared.get!('projectState.b')).toEqual({ history });
    shared.delete('projectHistory.a'); await shared.flushAll();
    shared.set('projectHistory.c', history);
    expect(shared.get!('projectHistory.c')).toEqual(history);
    expect(shared.get!('projectState.b')).toEqual({ history });
    await shared.flushAll();
    // Restore A for the independent cache-eviction/re-read test below.
    writer.set('projectHistory.a', history); await writer.flushAll();
    const store = createStore(directory, { heavyCacheBytes: 0 }); await store.load();
    for (let i = 0; i < 3; i++) {
      await store.preload!('projectHistory.a');
      expect(store.get!('projectState.b')).toEqual({ history });
      expect(store.get!('projectHistory.a')).toEqual(history);
    }
    store.set('projectState.b', { time: 3 }); await store.flushAll();
    expect(store.get!('projectState.b')).toEqual({ history, time: 3 });
  });

  it('atomically migrates legacy history on first access and preserves the original if the write fails', async () => {
    const directory = await temporaryDirectory();
    const original = JSON.stringify({ version: 1, index: -1, entries: [{ label: 'Add',
      forward: [{ path: ['layers'], exists: true, value: [{ id: 'one' }] }], backward: [],
    }] });
    const file = path.join(directory, 'projectHistory.legacy.json');
    await fs.writeFile(file, original);
    const broken = createStore(directory, { fs: { ...fs, rename: async () => { throw new Error('Disk unavailable'); } } });
    const errors: any[] = []; broken.onError(error => errors.push(error));
    await broken.load(); broken.get!('projectHistory.legacy'); await broken.flushAll();
    expect(await fs.readFile(file, 'utf8')).toBe(original);
    expect(errors).toHaveLength(1);
    expect(broken.get!('projectHistory.legacy')).toEqual(JSON.parse(original));
    const healthy = createStore(directory); await healthy.load(); expect(await fs.readFile(file, 'utf8')).toBe(original); await healthy.preload!('projectHistory.legacy'); await healthy.flushAll();
    expect(JSON.parse(await fs.readFile(file, 'utf8')).format).toBe('powermove-history-dictionary');
    const reopened = createStore(directory); await reopened.load();
    expect(reopened.get!('projectHistory.legacy')).toEqual(JSON.parse(original));
  });
  it('keeps inactive payloads out of bootstrap and preserves compact history across reopening', async () => {
    const directory = await temporaryDirectory(), store = createStore(directory);
    await store.load();
    const layer = { id: 'layer', d: { text: 'same pixels'.repeat(100) } };
    const history = { version: 1, index: 0, entries: Array.from({ length: 3 }, (_, i) => ({ label: `Edit ${i}`,
      forward: [{ path: ['layers'], exists: true, value: [structuredClone(layer)] }],
      backward: [{ path: ['layers'], exists: true, value: [structuredClone(layer)] }],
    })) };
    store.set('projectHistory.saved', history);
    store.set('agentAttachment.saved', { data: 'attachment bytes' });
    store.set('theme', 'dark');
    const bootstrap = store.bootstrapSerialized!();
    expect(bootstrap.theme).toBe('"dark"');
    expect(bootstrap['projectHistory.saved']).toBeUndefined();
    expect(bootstrap['agentAttachment.saved']).toBeUndefined();
    expect(JSON.parse(bootstrap.__powermoveLazyKeys!)).toEqual(['projectHistory.saved', 'agentAttachment.saved']);
    expect(JSON.parse(store.getEncodedSerialized!('projectHistory.saved')!).records).toHaveLength(1);
    await store.flushAll();
    const reopened = createStore(directory); await reopened.load();
    expect(reopened.get!('projectHistory.saved')).toEqual(history);
    const copy: any = reopened.get!('projectHistory.saved');
    copy.entries[0].forward[0].value[0].d.text = 'mutated';
    expect(reopened.get!('projectHistory.saved')).toEqual(history);
    expect(JSON.parse(reopened.getSerialized!('projectHistory.saved')!)).toEqual(history);
  });
  it('serializes individual reads and bootstrap without cloning unrelated project histories', async () => {
    const store = createStore(await temporaryDirectory());
    await store.load();
    store.set('projects', [{ id: 'demo' }]);
    store.set('projectHistory.demo', { entries: Array.from({ length: 1000 }, (_, i) => ({ i, label: 'Edit' })) });
    const clone = vi.spyOn(globalThis, 'structuredClone');
    expect(store.getSerialized!('projects')).toBe('[{"id":"demo"}]');
    expect(store.getSerialized!('theme')).toBeNull();
    expect(store.getSerialized!('../invalid')).toBeNull();
    const snapshot = store.snapshotSerialized!();
    expect(snapshot['projectHistory.demo']).toBeUndefined();
    expect(JSON.parse(snapshot.__powermoveLazyKeys!)).toContain('projectHistory.demo');
    expect(clone).not.toHaveBeenCalled();
    snapshot.projects = '[]';
    expect(store.getSerialized!('projects')).toBe('[{"id":"demo"}]');
    await store.flushAll();
  });

  it('preserves legacy undo history when a renderer updates metadata only', async () => {
    const directory = await temporaryDirectory(), store = createStore(directory);
    await store.load();
    const history = { version: 1, index: 0, entries: [{ label: 'Edit', forward: [], backward: [] }] };
    store.set('projectState.demo', { history, time: 1 });
    store.set('projectState.demo', { time: 2 });
    expect(store.get!('projectState.demo')).toEqual({ history, time: 2 });
    store.setSerialized!('projectState.demo', JSON.stringify({ time: 3 }));
    await store.flushAll();
    const reopened = createStore(directory); await reopened.load();
    expect(reopened.get!('projectState.demo')).toEqual({ history, time: 3 });
  });

  it('answers a read right after a metadata-only session write on this thread', async () => {
    const directory = await temporaryDirectory();
    const history = { version: 1, index: 0, entries: [{ label: 'Edit', forward: [], backward: [] }] };
    await fs.writeFile(path.join(directory, 'projectState.stored.json'), JSON.stringify({ history, time: 1 }));
    const store = createStore(directory);
    await store.load();
    // The `powermove serve` host reads synchronously in the same turn as the
    // write; the stored history must already be known, not read from disk.
    store.set('projectState.stored', { time: 2 });
    expect(JSON.parse(store.getSerialized!('projectState.stored')!)).toEqual({ history, time: 2 });
    store.set('projectState.fresh', { ok: true });
    expect(JSON.parse(store.getSerialized!('projectState.fresh')!)).toEqual({ ok: true });
    await store.flushAll();
    const reopened = createStore(directory); await reopened.load();
    expect(reopened.get!('projectState.stored')).toEqual({ history, time: 2 });
    expect(reopened.get!('projectState.fresh')).toEqual({ ok: true });
  });

  it('refuses a metadata-only session write whose stored history cannot be read, instead of dropping it', async () => {
    const directory = await temporaryDirectory();
    const history = { version: 1, index: 0, entries: [{ label: 'Edit', forward: [], backward: [] }] };
    await fs.writeFile(path.join(directory, 'projectState.stored.json'), JSON.stringify({ history, time: 1 }));
    let failRead = true;
    const readFileSync = ((...args: Parameters<typeof syncFs.readFileSync>) => {
      if (failRead && String(args[0]).endsWith('projectState.stored.json')) {
        failRead = false;
        throw Object.assign(new Error('Busy'), { code: 'EBUSY' });
      }
      return syncFs.readFileSync(...args);
    }) as typeof syncFs.readFileSync;
    const store = createStore(directory, { syncFs: { ...syncFs, readFileSync } });
    const errors: string[] = [];
    store.onError(event => errors.push(event.error));
    await store.load();
    // Sessions are not read at load: an unread one is unknown, not empty.
    expect(errors).toEqual([]);
    await expect(store.set('projectState.stored', { time: 2 })).rejects.toThrow(/could not be read/);
    expect(store.get!('projectState.stored')).toEqual({ history, time: 1 });
    await store.set('projectState.stored', { time: 3 });
    expect(store.get!('projectState.stored')).toEqual({ history, time: 3 });
    await store.flushAll();
    expect(JSON.parse(await fs.readFile(path.join(directory, 'projectState.stored.json'), 'utf8')).time).toBe(3);
  });

  it('round-trips set, snapshot, delete, and load without sharing references', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory);
    await store.load();

    const source = { nested: { count: 1 } };
    store.set('theme', source);
    source.nested.count = 99;
    expect(store.snapshot()).toEqual({ theme: { nested: { count: 1 } } });

    const exposed = store.snapshot() as { theme: { nested: { count: number } } };
    exposed.theme.nested.count = 50;
    expect(store.snapshot()).toEqual({ theme: { nested: { count: 1 } } });
    await store.flushAll();

    const reloaded = createStore(directory);
    await reloaded.load();
    expect(reloaded.snapshot()).toEqual({ theme: { nested: { count: 1 } } });

    reloaded.delete('theme');
    expect(reloaded.snapshot()).toEqual({});
    await reloaded.flushAll();
    await expect(fs.stat(path.join(directory, 'theme.json'))).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('coalesces five sets inside the 150 ms window into one write', async () => {
    const directory = await temporaryDirectory();
    const writeFile = vi.fn(fs.writeFile);
    const store = createStore(directory, { fs: fileSystemWith({ writeFile }) });
    await store.load();

    for (let value = 1; value <= 5; value += 1) store.set('theme', value);
    await store.flushAll();

    expect(writeFile).toHaveBeenCalledTimes(1);
    expect(JSON.parse(await fs.readFile(path.join(directory, 'theme.json'), 'utf8'))).toBe(5);
  });

  it('serializes interleaved writes per key so a slow older write cannot regress the file', async () => {
    const directory = await temporaryDirectory();
    let releaseFirst!: () => void;
    let markFirstStarted!: () => void;
    const firstStarted = new Promise<void>((resolve) => {
      markFirstStarted = resolve;
    });
    const firstMayFinish = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    let calls = 0;
    const writeFile: typeof fs.writeFile = async (...args: Parameters<typeof fs.writeFile>) => {
      calls += 1;
      if (calls === 1) {
        markFirstStarted();
        await firstMayFinish;
      }
      return fs.writeFile(...args);
    };
    const store = createStore(directory, {
      fs: fileSystemWith({ writeFile }),
      writeDelayMs: 0,
      randomSuffix: sequence('old', 'new')
    });
    await store.load();

    store.set('theme', 'old');
    await firstStarted;
    store.set('theme', 'new');
    const flushed = store.flushAll();
    await Promise.resolve();
    expect(calls).toBe(1);
    releaseFirst();
    await flushed;

    expect(calls).toBe(2);
    expect(JSON.parse(await fs.readFile(path.join(directory, 'theme.json'), 'utf8'))).toBe('new');
  });

  it('keeps the old destination intact when replacement fails before rename', async () => {
    const directory = await temporaryDirectory();
    const initial = createStore(directory);
    await initial.load();
    initial.set('theme', 'old');
    await initial.flushAll();

    const rename = vi.fn(async () => {
      throw new Error('injected rename failure');
    }) as unknown as typeof fs.rename;
    const events: Array<{ key: string; error: string }> = [];
    const failing = createStore(directory, {
      fs: fileSystemWith({ rename }),
      writeDelayMs: 0,
      randomSuffix: () => 'failure'
    });
    await failing.load();
    failing.onError((event) => events.push(event));
    failing.set('theme', 'new');
    await failing.flushAll();

    expect(JSON.parse(await fs.readFile(path.join(directory, 'theme.json'), 'utf8'))).toBe('old');
    expect(events).toContainEqual({ key: 'theme', error: 'injected rename failure' });
    await expect(fs.stat(path.join(directory, 'theme.json.tmp-failure'))).rejects.toMatchObject({
      code: 'ENOENT'
    });
  });

  it('writes, fsyncs, closes, and only then atomically renames', async () => {
    const directory = await temporaryDirectory();
    const calls: string[] = [];
    const writeFile: typeof fs.writeFile = async (...args: Parameters<typeof fs.writeFile>) => {
      calls.push('write');
      return fs.writeFile(...args);
    };
    const open: typeof fs.open = async (...args: Parameters<typeof fs.open>) => {
      calls.push('open');
      const handle = await fs.open(...args);
      const sync = handle.sync.bind(handle);
      const close = handle.close.bind(handle);
      handle.sync = async () => {
        calls.push('fsync');
        await sync();
      };
      handle.close = async () => {
        calls.push('close');
        await close();
      };
      return handle;
    };
    const rename: typeof fs.rename = async (...args: Parameters<typeof fs.rename>) => {
      calls.push('rename');
      return fs.rename(...args);
    };
    const store = createStore(directory, {
      fs: fileSystemWith({ writeFile, open, rename }),
      randomSuffix: () => 'sequence'
    });
    await store.load();
    store.set('theme', 'dark');
    await store.flushAll();

    expect(calls).toEqual(['write', 'open', 'fsync', 'close', 'rename']);
  });

  it('quarantines corrupt JSON and reports it without failing startup', async () => {
    const directory = await temporaryDirectory();
    await fs.writeFile(path.join(directory, 'theme.json'), '{"truncated":');
    const events: Array<{ key: string; error: string }> = [];
    const store = createStore(directory, { now: () => 1234 });
    store.onError((event) => events.push(event));

    await store.load();

    expect(store.snapshot()).toEqual({});
    await expect(fs.stat(path.join(directory, 'theme.json'))).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await fs.readFile(path.join(directory, 'theme.json.corrupt-1234'), 'utf8')).toBe('{"truncated":');
    expect(events[0]).toMatchObject({ key: 'theme' });
    expect(events[0]?.error).toContain('invalid store file quarantined');
  });

  it('removes stale sibling temporary files on startup', async () => {
    const directory = await temporaryDirectory();
    await fs.writeFile(path.join(directory, 'projects.json.tmp-stale'), 'partial');
    await fs.writeFile(path.join(directory, 'unrelated.txt'), 'keep');

    await createStore(directory).load();

    await expect(fs.stat(path.join(directory, 'projects.json.tmp-stale'))).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await fs.readFile(path.join(directory, 'unrelated.txt'), 'utf8')).toBe('keep');
  });

  it.each(['unknown', 'project.../etc', 'project.a/b', '../theme', 'project.']) (
    'rejects unknown or traversal-shaped key %s',
    async (key) => {
      const directory = await temporaryDirectory();
      const store = createStore(directory);
      await store.load();

      expect(() => store.set(key, true)).toThrow(IpcValidationError);
      expect(() => store.delete(key)).toThrow(IpcValidationError);
      expect(store.snapshot()).toEqual({});
    }
  );

  it('enforces the serialized UTF-8 byte limit before changing memory', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory);
    await store.load();
    const errors: Array<{ key: string; error: string }> = [];
    store.onError(event => errors.push(event));
    const tooLarge = 'x'.repeat(LIMITS.storeValueBytes);

    const refused = store.set('theme', tooLarge);
    // A synchronous read never shows the refused value, even before the engine answers.
    expect(store.snapshot()).toEqual({});
    await expect(refused).rejects.toThrow(IpcValidationError);
    // Fewer characters than the limit, more UTF-8 bytes: only the engine can tell.
    await expect(store.setSerialized!('theme', JSON.stringify('é'.repeat(LIMITS.storeValueBytes / 2)))).rejects.toThrow('serialized value exceeds its size limit');
    expect(() => store.setSerialized!('theme', 'x'.repeat(LIMITS.storeValueBytes + 1))).toThrow(IpcValidationError);
    await expect(store.setSerialized!('theme', '{"truncated":')).rejects.toThrow('invalid JSON');
    expect(store.snapshot()).toEqual({});
    expect(errors.map(event => event.key)).toEqual(['theme', 'theme', 'theme']);
    await store.flushAll();
    await expect(fs.stat(path.join(directory, 'theme.json'))).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('refuses values JSON cannot represent before they leave the calling thread', async () => {
    const store = createStore(await temporaryDirectory());
    await store.load();
    expect(() => store.set('theme', { render: () => 1 })).toThrow(IpcValidationError);
    await expect(store.set('theme', undefined)).rejects.toThrow('value is not JSON-serialisable');
    expect(store.snapshot()).toEqual({});
  });

  it('reports writes as failed, without throwing, once the storage worker is gone', async () => {
    let alive = true;
    let deliver!: (reply: StoreReply) => void;
    const channel: StoreChannel = {
      post(request) {
        if (!alive) throw new Error('The persistence worker stopped.');
        if (request.op === 'evict') return;
        const reply = request.op === 'init' ? { type: 'ready', id: request.id }
          : request.op === 'load' ? { type: 'loaded', id: request.id, entries: [], lazy: [] } : { type: 'flushed', id: request.id };
        queueMicrotask(() => deliver(reply as StoreReply));
      },
      onReply(listener) { deliver = listener; },
      waitSync: () => true
    };
    const store = createStore(await temporaryDirectory(), { thread: { storeChannel: () => channel } });
    await store.load();
    alive = false;
    const errors: Array<{ key: string; error: string }> = [];
    store.onError(event => errors.push(event));
    await expect(store.set('theme', 'dark')).rejects.toThrow('stopped');
    await expect(store.delete('theme')).rejects.toThrow('stopped');
    await expect(store.flushAll()).rejects.toThrow('stopped');
    expect(errors).toEqual([{ key: 'theme', error: 'The persistence worker stopped.' }, { key: 'theme', error: 'The persistence worker stopped.' }]);
    expect(store.snapshot()).toEqual({});
  });

  it('answers synchronous reads with the newest accepted value while writes are pending', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory, { writeDelayMs: 10_000 });
    await store.load();
    void store.set('projects', [{ id: 'one' }]);
    void store.setSerialized!('project.one', JSON.stringify({ v: '1', proj: { id: 'one', layers: [] } }));
    void store.delete('theme');
    expect(store.getSerialized!('projects')).toBe('[{"id":"one"}]');
    expect(JSON.parse(store.getEncodedSerialized!('project.one')!)).toEqual({ v: '1', proj: { id: 'one', layers: [] } });
    expect(JSON.parse(store.bootstrapSerialized!().__powermoveLazyKeys!)).toEqual(['project.one']);
    void store.delete('project.one');
    expect(JSON.parse(store.bootstrapSerialized!().__powermoveLazyKeys!)).toEqual([]);
    expect(store.getEncodedSerialized!('project.one')).toBeNull();
    await store.flushAll();
    expect(await fs.readdir(directory)).toEqual(['projects.json']);
  });

  it('round-trips the 59351487-byte project reported by the editor', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory);
    await store.load();
    const project = { id: 'large-project', editableSource: '' };
    project.editableSource = 'x'.repeat(59351487 - Buffer.byteLength(JSON.stringify(project)));
    store.set('project.large-project', project);
    await store.flushAll();
    expect((await fs.stat(path.join(directory, 'project.large-project.json'))).size).toBe(59351487);
    const reopened = createStore(directory);
    await reopened.load();
    expect(reopened.get!('project.large-project')).toEqual(project);
  }, 30_000);

  it('persists take history larger than the ordinary settings limit', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory);
    await store.load();
    const takes = [{ id: 'large-take', json: 'x'.repeat(LIMITS.storeValueBytes) }];
    store.set('takes', takes);
    await store.flushAll();
    const reopened = createStore(directory);
    await reopened.load();
    expect((reopened.snapshot().takes as typeof takes)[0]!.json.length).toBe(LIMITS.storeValueBytes);
  });

  it('round-trips the 33792768-byte project session reported by the editor', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory);
    await store.load();
    const state = {
      history: { version: 1, index: 0, entries: [{ label: 'Edit source',
        forward: [{ path: ['source'], exists: true, value: '' }],
        backward: [{ path: ['source'], exists: false }],
      }] },
      workspace: { name: 'Editing' }, time: 12,
    };
    state.history.entries[0]!.forward[0]!.value = 'x'.repeat(33792768 - Buffer.byteLength(JSON.stringify(state)));

    store.set('projectState.Pyckwzri', state);
    await store.flushAll();
    expect((await fs.stat(path.join(directory, 'projectState.Pyckwzri.json'))).size).toBe(33792768);
    const reopened = createStore(directory);
    await reopened.load();
    expect(reopened.get!('projectState.Pyckwzri')).toEqual(state);
  }, 30_000);

  it('round-trips the 168720601-byte take history reported by the editor', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory);
    await store.load();
    const takes = [{ id: 'reported-history', json: '' }];
    takes[0]!.json = 'x'.repeat(168720601 - Buffer.byteLength(JSON.stringify(takes)));
    store.set('takes', takes);
    await store.flushAll();
    expect((await fs.stat(path.join(directory, 'takes.json'))).size).toBe(168720601);
    const reopened = createStore(directory);
    await reopened.load();
    expect(reopened.snapshot().takes).toEqual(takes);
  }, 30_000);

  it('flushAll waits for an active write and a newer pending write', async () => {
    const directory = await temporaryDirectory();
    let release!: () => void;
    let started!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const didStart = new Promise<void>((resolve) => {
      started = resolve;
    });
    let first = true;
    const writeFile: typeof fs.writeFile = async (...args: Parameters<typeof fs.writeFile>) => {
      if (first) {
        first = false;
        started();
        await gate;
      }
      return fs.writeFile(...args);
    };
    const store = createStore(directory, { fs: fileSystemWith({ writeFile }), writeDelayMs: 0 });
    await store.load();
    store.set('theme', 1);
    await didStart;
    store.set('autosave', 2);

    let complete = false;
    const flushing = store.flushAll().then(() => {
      complete = true;
    });
    await Promise.resolve();
    expect(complete).toBe(false);
    release();
    await flushing;

    expect(JSON.parse(await fs.readFile(path.join(directory, 'theme.json'), 'utf8'))).toBe(1);
    expect(JSON.parse(await fs.readFile(path.join(directory, 'autosave.json'), 'utf8'))).toBe(2);
  });

  it('reports both sides of project registry/file reconciliation without deleting either', async () => {
    const directory = await temporaryDirectory();
    await fs.writeFile(path.join(directory, 'projects.json'), JSON.stringify([{ id: 'missing' }, { id: 'matched' }]));
    await fs.writeFile(path.join(directory, 'project.matched.json'), JSON.stringify({ id: 'matched' }));
    await fs.writeFile(path.join(directory, 'project.orphan.json'), JSON.stringify({ id: 'orphan' }));
    const events: Array<{ key: string; error: string }> = [];
    const store = createStore(directory);
    store.onError((event) => events.push(event));

    await store.load();

    expect(events).toEqual(
      expect.arrayContaining([
        { key: 'project.missing', error: 'project registry entry has no corresponding project file' },
        { key: 'project.orphan', error: 'project file has no corresponding registry entry' }
      ])
    );
    expect(store.snapshot().__powermoveLazyKeys).toContain('project.orphan');
    await expect(fs.stat(path.join(directory, 'project.orphan.json'))).resolves.toBeDefined();
  });
});

describe('store IPC and quit integration', () => {
  it('checks sender trust and returns validation errors on fire-and-forget mutations', () => {
    const handlers = new Map<string, (...args: any[]) => unknown>();
    const listeners = new Map<string, (...args: any[]) => unknown>();
    const ipcMain = {
      handle: (channel: string, handler: (...args: any[]) => unknown) => handlers.set(channel, handler),
      on: (channel: string, listener: (...args: any[]) => unknown) => {
        listeners.set(channel, listener);
        return ipcMain;
      }
    };
    const set = vi.fn((key: string) => {
      if (key === '../theme') throw new IpcValidationError(IPC.storeSet, `unknown store key: ${key}`);
    });
    const recovery = { 'projectHistory.demo': { undo: [{ layers: [{ id: 'one' }] }], redo: [] }, takes: [{ name: 'Saved', json: '{"layers":[]}' }] };
    const store = storeStub({ set, snapshot: () => recovery });
    registerStoreIpc(ipcMain as unknown as Pick<IpcMain, 'handle' | 'on'>, store, {
      isTrustedSender: (event) => Boolean((event as unknown as { trusted?: boolean }).trusted)
    });
    const send = vi.fn();
    const trustedEvent = { trusted: true, sender: { send }, returnValue: undefined };
    const untrustedEvent = { trusted: false, sender: { send }, returnValue: undefined };

    listeners.get(IPC.storeSnapshotSerializedSync)?.(trustedEvent);
    expect(Object.values(trustedEvent.returnValue as unknown as Record<string, string>).every(value => typeof value === 'string')).toBe(true);
    expect(Object.fromEntries(Object.entries(trustedEvent.returnValue as unknown as Record<string, string>)
      .map(([key, value]) => [key, JSON.parse(value)]))).toEqual({ ...recovery, __powermoveAsyncStore: true });
    listeners.get(IPC.storeSnapshotSerializedSync)?.(untrustedEvent);
    expect(untrustedEvent.returnValue).toEqual({});
    listeners.get(IPC.storeSnapshotSync)?.(trustedEvent);
    expect(trustedEvent.returnValue).toEqual({ ...recovery, __powermoveAsyncStore: true });

    listeners.get(IPC.storeSet)?.(untrustedEvent, { key: 'theme', value: 'dark' });
    listeners.get(IPC.storeSet)?.(trustedEvent, { key: '../theme', value: 'dark' });
    expect(set).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith(
      IPC.storeError,
      expect.objectContaining({ key: '../theme', error: expect.stringContaining('unknown store key') })
    );
    // Also exercise the boundary's malformed shape.
    listeners.get(IPC.storeSet)?.(trustedEvent, { value: 'dark' });
    expect(send).toHaveBeenCalledWith(
      IPC.storeError,
      expect.objectContaining({ key: '', error: expect.stringContaining('expected { key, value }') })
    );
    expect(() => handlers.get(IPC.storeSnapshot)?.(untrustedEvent)).toThrow(IpcValidationError);
  });

  it('tells sibling windows which keys moved, coalesced, and never the writer', async () => {
    const handlers = new Map<string, (...args: any[]) => unknown>();
    const listeners = new Map<string, (...args: any[]) => unknown>();
    const ipcMain = {
      handle: (channel: string, handler: (...args: any[]) => unknown) => handlers.set(channel, handler),
      on: (channel: string, listener: (...args: any[]) => unknown) => {
        listeners.set(channel, listener);
        return ipcMain;
      }
    };
    const broadcastChange = vi.fn();
    const store = storeStub({ snapshot: () => ({ projects: [{ id: 'P1' }] }),
      getEncodedSerialized: key => key === 'project.P1' ? '{"proj":{"id":"P1"}}' : null });
    registerStoreIpc(ipcMain as unknown as Pick<IpcMain, 'handle' | 'on'>, store, {
      isTrustedSender: (event) => Boolean((event as unknown as { trusted?: boolean }).trusted),
      broadcastChange
    });
    const sender = { send: vi.fn() };
    const event = { trusted: true, sender, returnValue: undefined as unknown };

    // A rename touches the registry and the slot; siblings hear about it once.
    listeners.get(IPC.storeSet)?.(event, { key: 'projects', value: [] });
    listeners.get(IPC.storeSet)?.(event, { key: 'project.P1', value: {} });
    listeners.get(IPC.storeDelete)?.(event, { key: 'projectJournal.P1' });
    expect(broadcastChange).not.toHaveBeenCalled();
    await nextTurn();
    expect(broadcastChange).toHaveBeenCalledTimes(1);
    expect(broadcastChange).toHaveBeenCalledWith(
      ['projects', 'project.P1', 'projectJournal.P1'],
      sender
    );

    // An untrusted write is not applied, so nothing is announced either.
    listeners.get(IPC.storeSet)?.({ trusted: false, sender }, { key: 'projects', value: [] });
    await nextTurn();
    expect(broadcastChange).toHaveBeenCalledTimes(1);

    // Refilling one invalidated key, and nothing at all for an untrusted asker.
    listeners.get(IPC.storeGetSync)?.(event, { key: 'projects' });
    expect(event.returnValue).toBe(JSON.stringify([{ id: 'P1' }]));
    listeners.get(IPC.storeGetSync)?.(event, { key: 'openWindows' });
    expect(event.returnValue).toBeNull();
    const untrusted = { trusted: false, sender, returnValue: undefined as unknown };
    listeners.get(IPC.storeGetSync)?.(untrusted, { key: 'projects' });
    expect(untrusted.returnValue).toBeNull();
    expect(await handlers.get(IPC.storeGetEncoded)?.(event, { key: 'project.P1' })).toBe('{"proj":{"id":"P1"}}');
    expect(await handlers.get(IPC.storeGetEncoded)?.(event, { key: 'project.missing' })).toBeNull();
    await expect(handlers.get(IPC.storeGetEncoded)?.(untrusted, { key: 'project.P1' })).rejects.toThrow(IpcValidationError);
    await expect(handlers.get(IPC.storeGetEncoded)?.(event, {})).rejects.toThrow(IpcValidationError);
  });

  it('announces a write only once it is encoded, and reports a refused one to its sender', async () => {
    const listeners = new Map<string, (...args: any[]) => unknown>();
    const handlers = new Map<string, (...args: any[]) => unknown>();
    const ipcMain = {
      handle: (channel: string, handler: (...args: any[]) => unknown) => handlers.set(channel, handler),
      on: (channel: string, listener: (...args: any[]) => unknown) => { listeners.set(channel, listener); return ipcMain; }
    };
    let accept!: () => void;
    const set = vi.fn((key: string) => key === 'theme'
      ? Promise.reject(new IpcValidationError(IPC.storeSet, 'serialised value is too large'))
      : new Promise<void>(resolve => { accept = resolve; }));
    const broadcastChange = vi.fn();
    registerStoreIpc(ipcMain as unknown as Pick<IpcMain, 'handle' | 'on'>, storeStub({ set, setSerialized: set }), { isTrustedSender: () => true, broadcastChange });
    const sender = { send: vi.fn(), isDestroyed: () => false };
    listeners.get(IPC.storeSet)?.({ sender }, { key: 'projects', value: [] });
    listeners.get(IPC.storeSet)?.({ sender }, { key: 'theme', value: 'x' });
    await nextTurn();
    expect(broadcastChange).not.toHaveBeenCalled();
    expect(sender.send).toHaveBeenCalledWith(IPC.storeError, { key: 'theme', error: `${IPC.storeSet}: serialised value is too large` });
    accept();
    await nextTurn();
    expect(broadcastChange).toHaveBeenCalledWith(['projects'], sender);
    await expect(handlers.get(IPC.storeSetSerialized)?.({ sender }, { key: 'theme', serialized: '"x"' })).rejects.toThrow('serialised value is too large');
    expect(broadcastChange).toHaveBeenCalledTimes(1);
  });

  it('prevents the first quit until flushing completes, then quits once more', async () => {
    const emitter = new EventEmitter();
    const quit = vi.fn();
    const app = Object.assign(emitter, { quit });
    let release!: () => void;
    const flushAll = vi.fn(() => new Promise<void>((resolve) => {
      release = resolve;
    }));
    installQuitFlush(app as unknown as Pick<App, 'on' | 'quit'>, storeStub({ flushAll }));
    const event = { preventDefault: vi.fn() };

    emitter.emit('before-quit', event);
    emitter.emit('before-quit', event);
    expect(event.preventDefault).toHaveBeenCalledTimes(2);
    expect(flushAll).toHaveBeenCalledTimes(1);
    expect(quit).not.toHaveBeenCalled();
    release();
    await vi.waitFor(() => expect(quit).toHaveBeenCalledTimes(1));
  });
  it('captures the live editor before flushing the main store',async()=>{
    const app=Object.assign(new EventEmitter(),{quit:vi.fn()});
    let prepared!:()=>void;
    const prepare=vi.fn(()=>new Promise<void>(resolve=>{prepared=resolve})),flushAll=vi.fn(async()=>{});
    const barrier=installQuitFlush(app as unknown as Pick<App,'on'|'quit'>,storeStub({flushAll}),prepare);
    app.emit('before-quit',{preventDefault:vi.fn()});expect(flushAll).not.toHaveBeenCalled();expect(barrier.isPrepared()).toBe(false);
    prepared();await vi.waitFor(()=>expect(app.quit).toHaveBeenCalledOnce());expect(flushAll).toHaveBeenCalledOnce();expect(barrier.isPrepared()).toBe(true);
  });
});

/** Promise callbacks, then the coalesced announcement (setImmediate). */
async function nextTurn(): Promise<void> {
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
}

function fileSystemWith(overrides: Partial<StoreFileSystem>): StoreFileSystem {
  return { ...fs, ...overrides } as StoreFileSystem;
}

function sequence(...values: string[]): () => string {
  let index = 0;
  return () => values[index++] ?? `extra-${index}`;
}

function storeStub(overrides: Partial<Store>): Store {
  return {
    load: async () => undefined,
    snapshot: () => ({}),
    set: () => undefined,
    delete: () => undefined,
    flushAll: async () => undefined,
    onError: () => () => undefined,
    ...overrides
  };
}
