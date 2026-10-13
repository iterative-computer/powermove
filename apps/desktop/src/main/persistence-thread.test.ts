import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';

import { IpcValidationError } from '../shared/guards';
import { PersistenceThread } from './persistence-thread';
import { createStore } from './storage';

/* The worker runs as built JavaScript, the way electron-vite ships it. */
let buildDirectory: string;
let entry: string;
const directories: string[] = [];
const threads: PersistenceThread[] = [];

beforeAll(async () => {
  buildDirectory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-persistence-build-'));
  entry = path.join(buildDirectory, 'persistence-worker.js');
  await build({
    entryPoints: [fileURLToPath(new URL('./persistence-worker.ts', import.meta.url))],
    outfile: entry, bundle: true, platform: 'node', format: 'cjs', logLevel: 'silent'
  });
}, 60_000);

afterAll(async () => { await fs.rm(buildDirectory, { recursive: true, force: true }); });

afterEach(async () => {
  await Promise.all(threads.splice(0).map(thread => thread.close()));
  await Promise.all(directories.splice(0).map(directory => fs.rm(directory, { recursive: true, force: true })));
});

async function temporaryDirectory(): Promise<string> {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-persistence-'));
  directories.push(directory);
  return directory;
}

function startThread(): PersistenceThread {
  const thread = new PersistenceThread(entry);
  threads.push(thread);
  return thread;
}

/** An undo history of unique layer records, the expensive shape to pack and serialize. */
function largeHistory(entries: number, layersPerEntry: number) {
  return {
    version: 1, index: entries - 1,
    entries: Array.from({ length: entries }, (_, entry) => ({
      label: `Edit ${entry}`,
      forward: [{ path: ['layers'], exists: true, value: Array.from({ length: layersPerEntry }, (_, i) => ({ id: `L${entry}-${i}`, name: `Layer ${entry}.${i}`, p: { x: { v: i, kf: Array.from({ length: 12 }, (_, k) => ({ t: k / 4, v: k * i })) } } })) }],
      backward: [{ path: ['layers'], exists: true, value: [] }]
    }))
  };
}

describe('store on the persistence worker thread', () => {
  it('round-trips values, history, and deletes to disk in the same format as before', async () => {
    const directory = await temporaryDirectory();
    const store = createStore(directory, { thread: startThread() });
    await store.load();
    const history = largeHistory(3, 4);
    // The same layer record repeated in Undo is stored once.
    history.entries[1]!.backward[0]!.value = structuredClone(history.entries[0]!.forward[0]!.value) as never;
    await store.set('projectHistory.demo', history);
    await store.setSerialized!('project.demo', JSON.stringify({ v: '1', proj: { id: 'demo', layers: [] } }));
    await store.set('theme', 'dark');
    await store.delete('theme');
    await store.flushAll();

    expect((await fs.readdir(directory)).sort()).toEqual(['project.demo.json', 'projectHistory.demo.json']);
    const packed = JSON.parse(await fs.readFile(path.join(directory, 'projectHistory.demo.json'), 'utf8'));
    expect(packed.format).toBe('powermove-history-dictionary');
    expect(packed.records).toHaveLength(12);

    const reopened = createStore(directory, { thread: startThread() });
    await reopened.load();
    expect(reopened.get!('projectHistory.demo')).toEqual(history);
    expect(JSON.parse(reopened.getSerialized!('projectHistory.demo')!)).toEqual(history);
    expect(reopened.get!('project.demo')).toEqual({ v: '1', proj: { id: 'demo', layers: [] } });
    expect(reopened.get!('theme')).toBeUndefined();
  });

  it('keeps main responsive while a large history is encoded, and reads wait for the newest value', async () => {
    const store = createStore(await temporaryDirectory(), { thread: startThread() });
    await store.load();
    const history = largeHistory(160, 120);
    const serialized = JSON.stringify(history);
    expect(serialized.length).toBeGreaterThan(5_000_000);

    let turns = 0, counting = true;
    const count = () => { if (counting) { turns++; setImmediate(count); } };
    setImmediate(count);
    const started = performance.now();
    const committed = store.setSerialized!('projectHistory.big', serialized);
    const callMs = performance.now() - started;
    await committed;
    counting = false;
    // Main only copied the string; parsing, packing, and serializing ran on the worker.
    expect(callMs).toBeLessThan(100);
    expect(turns).toBeGreaterThan(3);

    // A synchronous read right after a write blocks only until that value is encoded.
    void store.set('projectHistory.big', largeHistory(2, 2));
    expect(JSON.parse(store.getSerialized!('projectHistory.big')!)).toEqual(largeHistory(2, 2));
    await store.flushAll();
  });

  it('reports refused values without publishing them', async () => {
    const store = createStore(await temporaryDirectory(), { thread: startThread() });
    await store.load();
    await store.set('theme', 'dark');
    await expect(store.setSerialized!('theme', '{"broken":')).rejects.toThrow(IpcValidationError);
    await expect(store.set('theme', undefined)).rejects.toThrow('value is not JSON-serialisable');
    expect(() => store.set('theme', { render: () => 1 })).toThrow(IpcValidationError);
    expect(store.get!('theme')).toBe('dark');
  });

  it('keeps a metadata-only session update from erasing its stored undo history', async () => {
    const directory = await temporaryDirectory();
    await fs.writeFile(path.join(directory, 'projectState.demo.json'), JSON.stringify({ history: { version: 1, index: 0, entries: [{ label: 'Edit', forward: [], backward: [] }] }, time: 1 }));
    const store = createStore(directory, { thread: startThread() });
    await store.load();
    await store.set('projectState.demo', { time: 2 });
    expect(store.get!('projectState.demo')).toEqual({ history: { version: 1, index: 0, entries: [{ label: 'Edit', forward: [], backward: [] }] }, time: 2 });
    await store.flushAll();
    expect(JSON.parse(await fs.readFile(path.join(directory, 'projectState.demo.json'), 'utf8')).time).toBe(2);
  });

  it('replaces a stopped worker and writes every value it had not saved yet', async () => {
    const directory = await temporaryDirectory();
    const thread = startThread();
    const store = createStore(directory, { thread, writeDelayMs: 60_000 });
    const errors: string[] = [];
    store.onError(event => errors.push(`${event.key}: ${event.error}`));
    await store.load();
    await store.set('projects', [{ id: 'kept' }]);
    await store.set('theme', 'gone');
    await store.delete('theme');
    const before = thread.threadId;

    // Stop the worker as a fault would, with both writes still debounced.
    await (thread as unknown as { worker: { terminate(): Promise<number> } }).worker.terminate();
    await expect.poll(() => thread.threadId).not.toBe(before);
    await store.flushAll();

    expect(JSON.parse(await fs.readFile(path.join(directory, 'projects.json'), 'utf8'))).toEqual([{ id: 'kept' }]);
    await expect(fs.stat(path.join(directory, 'theme.json'))).rejects.toMatchObject({ code: 'ENOENT' });
    expect(store.get!('projects')).toEqual([{ id: 'kept' }]);
    await store.set('theme', 'after');
    await store.flushAll();
    expect(JSON.parse(await fs.readFile(path.join(directory, 'theme.json'), 'utf8'))).toBe('after');
    expect(errors).toEqual([]);
  });

  it('checks a saved project document on the worker', async () => {
    const directory = await temporaryDirectory();
    const thread = startThread();
    const file = path.join(directory, 'transaction');
    const document = JSON.stringify({ proj: { w: 10, h: 10, layers: [] } });
    await fs.writeFile(file, `PMV4${document}tail`);
    await expect(thread.validateProjectDocument(file, 4, document.length)).resolves.toBeUndefined();
    await expect(thread.validateProjectDocument(file, 4, document.length - 1)).rejects.toThrow(/JSON/);
    await fs.writeFile(file, '{"proj":{"layers":[]}}');
    await expect(thread.validateProjectDocument(file, 0, 22)).rejects.toThrow('This is not a Powermove project.');
  });
});
