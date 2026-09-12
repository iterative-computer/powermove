import { EventEmitter } from 'node:events';
import * as fs from 'node:fs/promises';
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
  type StoreFileSystem
} from './storage';

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
  it('preserves legacy undo history when a renderer updates metadata only', async () => {
    const directory = await temporaryDirectory(), store = createStore(directory);
    await store.load();
    const history = { version: 1, index: 0, entries: [{ label: 'Edit', forward: [], backward: [] }] };
    store.set('projectState.demo', { history, time: 1 });
    store.set('projectState.demo', { time: 2 });
    expect(store.snapshot()['projectState.demo']).toEqual({ history, time: 2 });
    store.setSerialized!('projectState.demo', JSON.stringify({ time: 3 }));
    await store.flushAll();
    const reopened = createStore(directory); await reopened.load();
    expect(reopened.snapshot()['projectState.demo']).toEqual({ history, time: 3 });
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
    const tooLarge = 'x'.repeat(LIMITS.storeValueBytes);

    expect(() => store.set('theme', tooLarge)).toThrow(IpcValidationError);
    expect(store.snapshot()).toEqual({});
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
    expect(reopened.snapshot()['project.large-project']).toEqual(project);
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
    expect(reopened.snapshot()['projectState.Pyckwzri']).toEqual(state);
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
    expect(store.snapshot()).toHaveProperty('project.orphan');
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
