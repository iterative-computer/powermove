// @vitest-environment happy-dom
/*
 * OPENWRITE-1 crash safety. The real window store (electron-shim), project
 * registry (core/projects) and main's store engine (main/storage, on this
 * thread) run an open's checkpoint sequence against a real directory. Every
 * mutating file operation is logged; afterwards the directory is rebuilt as it
 * stood after each prefix of that log (a crash between any two operations),
 * reloaded through a fresh store, and the project recovered by a fresh
 * window's Projects registry (Projects.get: slot plus journal). Every crash point must
 * recover nothing or one complete state the document actually had, never an
 * older state than an earlier crash point, and at every quiescent point
 * exactly the state today's code recovers there (today wrote the whole
 * document at every one of these requests).
 */
import * as nodeFs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { bridge as currentBridge, installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import type { PMRegistry } from '../registry';
import { install as installShim } from '../host/electron-shim';
import { install as installProjects } from '../core/projects';

/* Main's store, loaded at run time: this file type-checks with the renderer,
   which does not include main's sources. */
type StoreFileSystem = any;
type Store = {
  load(): Promise<void>; onError?(cb: (event: { key: string; error: string }) => void): () => void; get?(key: string): unknown; getEncodedSerialized?(key: string): string | null;
  bootstrapSerialized?(): Record<string, string>; set(key: string, value: unknown): void | Promise<void>;
  setSerialized?(key: string, serialized: string): void | Promise<void>; appendJournal?(key: string, serialized: string): Promise<void>;
  delete(key: string): void | Promise<void>; flushAll(): Promise<void>;
};
const MAIN_STORAGE = '../../../../main/storage';
let createStore: (directory: string, options?: Record<string, unknown>) => Store;

type Op = { kind: 'write'; file: string; data: string } | { kind: 'rename'; from: string; to: string } | { kind: 'unlink'; file: string };

const directories: string[] = [];
async function temporaryDirectory(): Promise<string> {
  const directory = await nodeFs.mkdtemp(path.join(os.tmpdir(), 'powermove-checkpoint-crash-'));
  directories.push(directory);
  return directory;
}
beforeAll(async () => {
  ({ createStore } = await import(/* @vite-ignore */ MAIN_STORAGE));
});
afterEach(async () => {
  vi.unstubAllGlobals();
  resetBridgeForTests();
  await Promise.all(directories.splice(0).map(directory => nodeFs.rm(directory, { recursive: true, force: true })));
});

/** Failures to inject: return an error to fail that operation (before it changes anything). */
type Faults = { writeFile?: (file: string) => Error | null; rename?: (to: string) => Error | null; gate?: (to: string) => Promise<void> | null };
const failure = (code: string, message: string) => Object.assign(new Error(`${code}: ${message}`), { code });

/** The file system, logging every operation that changes what a restart reads. */
function loggingFs(log: Op[], faults: Faults = {}): StoreFileSystem {
  return {
    ...nodeFs,
    writeFile: (async (file: any, data: any, options: any) => {
      const fault = faults.writeFile?.(String(file));
      if (fault) throw fault;
      await nodeFs.writeFile(file, data, options);
      log.push({ kind: 'write', file: String(file), data: typeof data === 'string' ? data : Buffer.from(data).toString('utf8') });
    }) as any,
    rename: (async (from: any, to: any) => {
      await faults.gate?.(String(to));
      const fault = faults.rename?.(String(to));
      if (fault) throw fault;
      await nodeFs.rename(from, to);
      log.push({ kind: 'rename', from: String(from), to: String(to) });
    }) as any,
    unlink: (async (file: any) => {
      await nodeFs.unlink(file);
      log.push({ kind: 'unlink', file: String(file) });
    }) as any,
  } as StoreFileSystem;
}

/** Where two JSON documents differ: up to `limit` paths with short values (whole documents are megabytes). */
function jsonDiff(a: string | null, b: string | null, limit = 8): string[] {
  if (a === b) return [];
  if (a === null || b === null) return [`(document) ${a === null ? 'absent' : 'present'} -> ${b === null ? 'absent' : 'present'}`];
  const out: string[] = [];
  const short = (value: unknown) => value === undefined ? 'absent' : JSON.stringify(value).slice(0, 80);
  const walk = (x: any, y: any, at: string) => {
    if (out.length >= limit || Object.is(x, y)) return;
    if (!x || !y || typeof x !== 'object' || typeof y !== 'object' || Array.isArray(x) !== Array.isArray(y)) { out.push(`${at || '/'}: ${short(x)} -> ${short(y)}`); return; }
    for (const key of new Set([...Object.keys(x), ...Object.keys(y)])) walk(x[key], y[key], `${at}/${key}`);
  };
  walk(JSON.parse(a), JSON.parse(b), '');
  return out.length ? out : ['(same values, different JSON text)'];
}

/** The store directory as it was after the first `count` logged operations. */
function stateAfter(log: Op[], count: number): Map<string, string> {
  const files = new Map<string, string>();
  for (const op of log.slice(0, count)) {
    if (op.kind === 'write') files.set(path.basename(op.file), op.data);
    else if (op.kind === 'rename') { const data = files.get(path.basename(op.from)); files.delete(path.basename(op.from)); if (data !== undefined) files.set(path.basename(op.to), data); }
    else files.delete(path.basename(op.file));
  }
  return files;
}

/** What a restart recovers for `id`: a fresh window's Projects.get (the slot with its journal replayed).
 *  Call it only after the window under test is done: it installs a window of its own. */
async function recover(files: Map<string, string>, id: string): Promise<string | null> {
  const directory = await temporaryDirectory();
  for (const [name, data] of files) await nodeFs.writeFile(path.join(directory, name), data);
  const store = createStore(directory);
  await store.load();
  const window = (globalThis as any).window, bridge = currentBridge();
  try {
    const PM: any = openWindow(store, { transient: true });
    const project = PM.Projects.get(id);
    return project ? JSON.stringify(project) : null;
  } finally {
    (globalThis as any).window = window;
    if (bridge) installBridgeForTests(bridge as any);
    await nodeFs.rm(directory, { recursive: true, force: true });
  }
}

/** A window whose store reaches `store` the way preload and main's IPC do. */
function openWindow(store: Store, options: { refuse?: (key: string) => boolean; transient?: boolean } = {}) {
  // vi.fn keeps every call's arguments (the shim's listeners, so the whole window) until the file ends.
  const spy = (): any => options.transient ? () => undefined : vi.fn();
  const listeners = new Map<string, Array<(payload: any) => void>>();
  const PM: PMRegistry = {
    uid: (prefix: string) => `${prefix}1`, toast: spy(), version: '1.0.0',
    bus: {
      on: (event: string, listener: (payload: any) => void) => { listeners.set(event, [...listeners.get(event) ?? [], listener]); return () => {}; },
      emit: (event: string, payload?: any) => { for (const listener of listeners.get(event) ?? []) listener(payload); },
    },
    CodexBridge: { progress: spy(), trace: spy(), resolve: spy() }, AgentArtifacts: { resolve: spy() }, WindowCapture: { resolve: spy() },
    store: {},
  };
  const bridge: any = {
    store: {
      bootstrapSerializedSync: () => ({ ...store.bootstrapSerialized!(), __powermoveAsyncStore: 'true' }),
      getEncodedSync: (key: string) => store.getEncodedSerialized!(key),
      // A refusal reaches the window as the invoke's rejection (main's storage IPC).
      setSerialized: (key: string, serialized: string) => store.setSerialized!(key, options.refuse?.(key) ? '{refused' : JSON.parse(JSON.stringify(serialized))),
      // Main's journal (SAVE-1), where the store has one.
      ...(store.appendJournal ? { appendJournal: (key: string, serialized: string) => store.appendJournal!(key, JSON.parse(JSON.stringify(serialized))) } : {}),
      set: (key: string, value: unknown) => { void Promise.resolve(store.set(key, structuredClone(value))).catch(() => undefined); },
      delete: (key: string) => { void Promise.resolve(store.delete(key)).catch(() => undefined); },
      // As main's storage IPC: a flush names the keys still not on disk.
      flush: async () => { await store.flushAll(); return { failing: (store as any).failingKeys?.() ?? [] }; },
      // As main's storage IPC: a write that did not reach disk is reported to its writer (this window).
      onError: (cb: (event: { key: string; error: string; unsaved?: true }) => void) => store.onError?.((event: any) => { if (event.unsaved) cb({ key: event.key, error: event.error, unsaved: true }); }) ?? (() => {}),
    },
    setTheme: spy(), log: spy(), onMenuCommand: spy(),
    codex: { run: spy(), cancel: spy(), requestComputerConsent: spy() }, artifacts: { read: spy(), reveal: spy() }, captureWindow: spy(), saveFile: spy(),
  };
  // A transient window (a recovery) is not registered with vi.stubGlobal, whose record of each replaced value kept every one alive.
  (options.transient ? (value: unknown) => { (globalThis as any).window = value; } : (value: unknown) => vi.stubGlobal('window', value))({
    powermove: bridge, TextEncoder, structuredClone, console, setTimeout,
    atob: (value: string) => Buffer.from(value, 'base64').toString('binary'),
    btoa: (value: string) => Buffer.from(value, 'binary').toString('base64'),
    document: { readyState: 'loading', addEventListener: spy(), documentElement: { classList: { add: spy() } } },
    addEventListener: spy(),
  });
  installBridgeForTests(bridge);
  installShim(PM);
  installProjects(PM);
  return PM;
}

/** A project above the size at which repeated checkpoints are checked rather than rewritten. */
function largeProject(id: string) {
  const layers = Array.from({ length: 24 }, (_, index) => ({
    id: `L${index}`, name: `Layer ${index}`,
    p: Object.fromEntries(['x', 'y', 'r', 's', 'o', 'w', 'h'].map(name => [name, Array.from({ length: 700 }, (_, key) => ({ t: key / 24, v: Math.round(Math.sin(key * (index + 1)) * 1e6) / 1e3, eo: [0.33, 0], ei: [0.67, 1] }))])),
  }));
  return { id, name: 'Large', w: 1920, h: 1080, revision: 7, assets: { a1: { id: 'a1', kind: 'image', w: 640, h: 480 } } as Record<string, any>, layers: layers as any[] };
}

async function settle(PM: any, store: Store, options: { failing?: boolean } = {}) {
  for (let round = 0; round < 3; round++) {
    // While a write is failing, the window's flush reports it (checked separately).
    if (options.failing) await PM.store.flush().catch(() => undefined);
    else await PM.store.flush();
    await store.flushAll();
    await new Promise(resolve => setTimeout(resolve, 5));
  }
}

describe('open-time project checkpoints', () => {
  it('recover a complete state of the document at every crash point, as today does once settled', { timeout: 120_000 }, async () => {
    const directory = await temporaryDirectory();
    const log: Op[] = [];
    const store = createStore(directory, { fs: loggingFs(log), writeDelayMs: 0 });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('P');
    PM.proj = project;
    const states: string[] = [];
    /** Each settled step: the log position, and the state today's code recovers there. */
    const settled: Array<{ at: number; state: string }> = [];
    const step = async (change?: () => void) => {
      await settle(PM, store);
      states.push(JSON.stringify(project));
      settled.push({ at: log.length, state: JSON.stringify(project) });
      void change;
    };
    const slotWrites = () => log.filter(op => op.kind === 'rename' && path.basename(op.to) === 'project.P.json').length;

    // switchProject: the opened document's first checkpoint.
    PM.Projects.put(project);
    await step();
    expect(JSON.parse(states[0]!).layers.length).toBe(24);
    // The autosave switchProject schedules (a recovery copy with a thumbnail).
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    await step();
    // The poster backfill: asset metadata changes outside typed history.
    project.assets.a1!.poster = true;
    PM.Projects.checkpointAssets(project);
    await step();
    // captureProjectSession as the open finishes, and the autosave after it.
    PM.Projects.put(project);
    PM.Projects.recover(project);
    await step();
    expect(slotWrites()).toBe(1);
    // An edit through history: journaled.
    project.layers[0].name = 'Renamed';
    PM.bus.emit('history:project-patch', { projectId: 'P', revision: 8, patches: [{ path: ['layers', 0, 'name'], exists: true, value: 'Renamed' }] });
    await step();
    // A raw write nothing announced, then a session capture (quit, tab switch): written.
    project.layers[1].name = 'Raw';
    PM.Projects.put(project);
    await step();
    expect(slotWrites()).toBe(2);
    // The same capture again: checked, nothing to write.
    PM.Projects.put(project);
    await step();
    expect(slotWrites()).toBe(2);
    // A raw write before a capture whose check finds it.
    project.layers[2].p.x[3].v = 42;
    PM.Projects.put(project);
    await step();
    expect(slotWrites()).toBe(3);
    // An announced edit while a check reads the document: rewritten with it.
    PM.Projects.put(project);
    project.layers[3].name = 'During the check';
    PM.bus.emit('history:project-patch', { projectId: 'P', revision: 9, patches: [{ path: ['layers', 3, 'name'], exists: true, value: 'During the check' }] });
    await step();

    // Settled points recover exactly the current document, as today's code does.
    for (const { at, state } of settled) expect(jsonDiff(state, await recover(stateAfter(log, at), 'P')), `settled at operation ${at}`).toEqual([]);
    // Every crash point: nothing, or a complete state, never older than before.
    let newest = -1;
    for (let count = 0; count <= log.length; count++) {
      const recovered = await recover(stateAfter(log, count), 'P');
      if (recovered === null) { expect(newest).toBe(-1); continue; }
      const index = states.lastIndexOf(recovered);
      expect(index, `crash after operation ${count} recovered a state the document never had`).toBeGreaterThanOrEqual(0);
      expect(index, `crash after operation ${count} recovered an older state`).toBeGreaterThanOrEqual(newest);
      newest = index;
    }
  });

  it('offers the JSON it just wrote only for the rest of that task and until a change is announced', async () => {
    const store = createStore(await temporaryDirectory(), { writeDelayMs: 0 });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('W');
    PM.proj = project;
    PM.Projects.put(project);
    expect(PM.Projects.writtenJSON(project)).toBe(JSON.stringify(project));
    expect(PM.Projects.writtenJSON({ ...project })).toBeNull();
    await Promise.resolve();
    expect(PM.Projects.writtenJSON(project)).toBeNull();
    project.name = 'changed';
    PM.Projects.put(project);
    PM.bus.emit('layers');
    expect(PM.Projects.writtenJSON(project)).toBeNull();
    await settle(PM, store);
  });

  it('writes a small project directly at each checkpoint, as before', async () => {
    const directory = await temporaryDirectory();
    const log: Op[] = [];
    const store = createStore(directory, { fs: loggingFs(log), writeDelayMs: 0 });
    await store.load();
    const PM: any = openWindow(store);
    const project = { id: 'S', name: 'Small', w: 10, h: 10, revision: 1, layers: [{ id: 'one' }] };
    PM.proj = project;
    PM.Projects.put(project);
    await settle(PM, store);
    project.layers.push({ id: 'two' });
    PM.Projects.put(project);
    await settle(PM, store);
    expect(log.filter(op => op.kind === 'rename' && path.basename(op.to) === 'project.S.json')).toHaveLength(2);
    expect(jsonDiff(JSON.stringify(project), await recover(stateAfter(log, log.length), 'S'))).toEqual([]);
  });
});

/** Every crash point of `log` recovers nothing (only before the first checkpoint), or one of
 *  `states`, never older than an earlier crash point did: no durable edit is lost. */
async function expectEveryCrashPointSafe(log: Op[], states: string[], id: string) {
  let newest = -1;
  for (let count = 0; count <= log.length; count++) {
    const recovered = await recover(stateAfter(log, count), id);
    if (recovered === null) { expect(newest, `crash after operation ${count} lost the project`).toBe(-1); continue; }
    const index = states.lastIndexOf(recovered);
    if (index < 0) expect(jsonDiff(states[Math.max(newest, 0)]!, recovered), `crash after operation ${count} recovered a state the document never had`).toEqual([]);
    if (index < newest) expect(jsonDiff(states[newest]!, recovered), `crash after operation ${count} recovered an older state`).toEqual([]);
    newest = index;
  }
}

describe('a project checkpoint that fails to reach disk', () => {
  const edit = (PM: any, project: any, revision: number, layer: number, name: string) => {
    project.revision = revision;
    project.layers[layer].name = name;
    PM.bus.emit('history:project-patch', { projectId: project.id, revision, patches: [{ path: ['layers', layer, 'name'], exists: true, value: name }, { path: ['revision'], exists: true, value: revision }] });
  };

  it('keeps the journal when the slot rename fails, and the next autosave writes the slot again', { timeout: 120_000 }, async () => {
    const log: Op[] = [];
    let failRename = false;
    const store = createStore(await temporaryDirectory(), {
      fs: loggingFs(log, { rename: to => failRename && path.basename(to) === 'project.F.json' ? failure('EIO', 'i/o error, rename') : null }),
      writeDelayMs: 0,
    });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('F');
    PM.proj = project;
    const states: string[] = [];
    const marks: Record<string, number> = {};
    const step = async (name: string, failing = false) => {
      await settle(PM, store, { failing });
      states.push(JSON.stringify(project));
      marks[name] = log.length;
    };
    const slotWrites = () => log.filter(op => op.kind === 'rename' && path.basename(op.to) === 'project.F.json').length;

    PM.Projects.put(project);
    await step('opened');
    edit(PM, project, 8, 0, 'Journaled');
    await step('journaled');
    // The session capture's checkpoint: posted, but its rename fails.
    failRename = true;
    PM.Projects.put(project);
    await step('slot failed', true);
    failRename = false;
    expect(slotWrites()).toBe(1);
    await expect(PM.store.flush()).rejects.toThrow(/EIO/);
    // An edit while the slot on disk is the older one.
    edit(PM, project, 9, 1, 'After the failure');
    await step('edited after', true);
    // The next autosave (it has journal entries) repairs it.
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    await step('autosaved', true);
    const healed = slotWrites();
    const flushed = await PM.store.flush().then(() => 'flushed', (error: Error) => error.message);
    // And checks, rather than rewrites, from then on.
    PM.Projects.put(project);
    await step('checked', true);

    // The journaled edit survives the failed checkpoint (the older slot plus its journal) ...
    expect(jsonDiff(states[1]!, await recover(stateAfter(log, marks['slot failed']!), 'F')), 'after the failed checkpoint').toEqual([]);
    // ... and the next autosave writes the slot again, which clears the error.
    expect(jsonDiff(states[3]!, await recover(stateAfter(log, marks['autosaved']!), 'F')), 'after the next autosave').toEqual([]);
    expect(healed).toBe(2);
    expect(flushed).toBe('flushed');
    expect(slotWrites()).toBe(2);
    await expectEveryCrashPointSafe(log, states, 'F');
  });

  it('writes a slot whose earlier write ran out of space, though the document matches what was posted', { timeout: 120_000 }, async () => {
    const log: Op[] = [];
    let full = false;
    const store = createStore(await temporaryDirectory(), {
      fs: loggingFs(log, { writeFile: file => full && path.basename(file).startsWith('project.E.json.tmp-') ? failure('ENOSPC', 'no space left on device, write') : null }),
      writeDelayMs: 0,
    });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('E');
    PM.proj = project;
    const states: string[] = [];
    const marks: Record<string, number> = {};
    const step = async (name: string, failing = false) => {
      await settle(PM, store, { failing });
      states.push(JSON.stringify(project));
      marks[name] = log.length;
    };
    const slotWrites = () => log.filter(op => op.kind === 'rename' && path.basename(op.to) === 'project.E.json').length;

    PM.Projects.put(project);
    await step('opened');
    // A raw write a capture's check finds: the slot is written, and the disk is full.
    full = true;
    project.layers[1].name = 'Raw';
    PM.Projects.put(project);
    await step('slot failed', true);
    full = false;
    expect(slotWrites()).toBe(1);
    await expect(PM.store.flush()).rejects.toThrow(/ENOSPC/);
    // The next autosave (no journal entries: the same object, unchanged since it was posted).
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    await step('autosaved', true);
    const flushed = await PM.store.flush().then(() => 'flushed', (error: Error) => error.message);

    // The raw write reaches disk at the next autosave, which clears the error.
    expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, marks['autosaved']!), 'E')), 'after the next autosave').toEqual([]);
    expect(slotWrites()).toBe(2);
    expect(flushed).toBe('flushed');
    await expectEveryCrashPointSafe(log, states, 'E');
  });
});

describe('periodic checkpoints of a large project', () => {
  const edit = (PM: any, project: any, revision: number, layer: number, name: string) => {
    project.revision = revision;
    project.layers[layer].name = name;
    PM.bus.emit('history:project-patch', { projectId: project.id, revision, patches: [{ path: ['layers', layer, 'name'], exists: true, value: name }, { path: ['revision'], exists: true, value: revision }] });
  };

  it('are serialized in slices and posted only if nothing changed while they were read', { timeout: 120_000 }, async () => {
    const log: Op[] = [];
    const store = createStore(await temporaryDirectory(), { fs: loggingFs(log), writeDelayMs: 0 });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('W');
    PM.proj = project;
    const states: string[] = [];
    const marks: Record<string, number> = {};
    const step = async (name: string) => {
      await settle(PM, store);
      states.push(JSON.stringify(project));
      marks[name] = log.length;
    };
    const slotWrites = () => log.filter(op => op.kind === 'rename' && path.basename(op.to) === 'project.W.json').length;
    const set = vi.spyOn(PM.store, 'set');

    PM.Projects.put(project);
    await step('opened');
    edit(PM, project, 49, 0, 'Forty-nine');
    PM.Projects.recover(project);
    await step('journaled');
    expect(slotWrites()).toBe(1);
    // The fiftieth revision checkpoints; an edit lands while it is read.
    edit(PM, project, 50, 1, 'Fifty');
    set.mockClear();
    PM.Projects.recover(project);
    edit(PM, project, 51, 2, 'While read');
    await step('changed while read');
    // Never serialized in one task, and not posted: the journal holds both edits.
    expect(set.mock.calls.filter(([key]) => key === 'project.W')).toHaveLength(0);
    expect(slotWrites()).toBe(1);
    // Still due: the next autosave checkpoints, though 51 is no multiple of 50.
    PM.Projects.recover(project);
    await step('checkpointed');
    expect(set.mock.calls.filter(([key]) => key === 'project.W')).toHaveLength(0);
    expect(slotWrites()).toBe(2);
    // And no more until the next one is due.
    edit(PM, project, 52, 3, 'After');
    PM.Projects.recover(project);
    await step('journaled after');
    expect(slotWrites()).toBe(2);

    for (const name of ['changed while read', 'checkpointed', 'journaled after']) {
      expect(jsonDiff(states[Object.keys(marks).indexOf(name)]!, await recover(stateAfter(log, marks[name]!), 'W')), name).toEqual([]);
    }
    await expectEveryCrashPointSafe(log, states, 'W');
  });
});

describe('edits while a checkpoint is failing', () => {
  const edit = (PM: any, project: any, revision: number, layer: number, name: string) => {
    project.revision = revision;
    project.layers[layer].name = name;
    PM.bus.emit('history:project-patch', { projectId: project.id, revision, patches: [{ path: ['layers', layer, 'name'], exists: true, value: name }, { path: ['revision'], exists: true, value: revision }] });
  };

  it('are on disk as they are made, through a failing slot write (D1)', { timeout: 120_000 }, async () => {
    const log: Op[] = [];
    let failRename = false;
    const store = createStore(await temporaryDirectory(), {
      fs: loggingFs(log, { rename: to => failRename && path.basename(to) === 'project.D.json' ? failure('EIO', 'i/o error, rename') : null }),
      writeDelayMs: 0,
    });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('D');
    PM.proj = project;
    const states: string[] = [];
    const step = async (failing = false) => { await settle(PM, store, { failing }); states.push(JSON.stringify(project)); return log.length; };

    PM.Projects.put(project);
    await step();
    edit(PM, project, 8, 0, 'Before');
    await step();
    failRename = true;
    PM.Projects.put(project);
    await step(true);
    // Edits while every slot write fails: each is on disk once settled.
    const marks: number[] = [];
    for (const [index, name] of ['First', 'Second', 'Third'].entries()) {
      edit(PM, project, 9 + index, 1 + index, name);
      marks.push(await step(true));
    }
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    marks.push(await step(true));
    for (const [index, at] of marks.entries()) {
      expect(jsonDiff(states[3 + index]!, await recover(stateAfter(log, at), 'D')), `settled edit ${index}`).toEqual([]);
    }
    failRename = false;
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    const healed = await step();
    expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, healed), 'D')), 'after the disk recovers').toEqual([]);
    await expectEveryCrashPointSafe(log, states, 'D');
  });

  it('a checkpoint refused while the one before it is still being written loses no edit (review probe)', { timeout: 120_000 }, async () => {
    const log: Op[] = [];
    let gate: Promise<void> | null = null;
    let release = () => {};
    let refuse = false;
    const store = createStore(await temporaryDirectory(), {
      fs: loggingFs(log, { gate: to => path.basename(to) === 'project.Q.json' ? gate : null }),
      writeDelayMs: 0,
    });
    await store.load();
    const PM: any = openWindow(store, { refuse: key => refuse && key === 'project.Q' });
    const project = largeProject('Q');
    PM.proj = project;
    const states: string[] = [];
    const step = async (failing = false) => { await settle(PM, store, { failing }); states.push(JSON.stringify(project)); return log.length; };
    const tick = () => new Promise(resolve => setTimeout(resolve, 20));

    PM.Projects.put(project);
    await step();
    // Checkpoint A, whose rename waits; an edit against it.
    gate = new Promise<void>(resolve => { release = resolve; });
    edit(PM, project, 8, 0, 'Before A');
    states.push(JSON.stringify(project));
    PM.Projects.put(project);
    await tick();
    edit(PM, project, 9, 1, 'Against A');
    await tick();
    states.push(JSON.stringify(project));
    // Checkpoint B is refused (the project crossed the size limit); an edit after it.
    refuse = true;
    PM.Projects.put(project);
    await tick();
    edit(PM, project, 10, 2, 'After B');
    await tick();
    states.push(JSON.stringify(project));
    release();
    const settled = await step(true);
    expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, settled), 'Q')), 'settled while refused').toEqual([]);
    refuse = false;
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    await step();
    expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, log.length), 'Q')), 'after the next autosave').toEqual([]);
    await expectEveryCrashPointSafe(log, states, 'Q');
  });
});

describe('a raw write after a journaled edit', () => {
  it('to the same path is not reverted by the journal at any crash point', { timeout: 120_000 }, async () => {
    const log: Op[] = [];
    const store = createStore(await temporaryDirectory(), { fs: loggingFs(log), writeDelayMs: 0 });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('R');
    PM.proj = project;
    const states: string[] = [];
    const step = async () => { await settle(PM, store); states.push(JSON.stringify(project)); };

    PM.Projects.put(project);
    await step();
    project.revision = 8;
    project.layers[4].name = 'Journaled';
    PM.bus.emit('history:project-patch', { projectId: 'R', revision: 8, patches: [{ path: ['layers', 4, 'name'], exists: true, value: 'Journaled' }, { path: ['revision'], exists: true, value: 8 }] });
    await step();
    // Unannounced: the same member, and another one, then a capture.
    project.layers[4].name = 'Raw';
    project.layers[5].name = 'Raw too';
    PM.Projects.put(project);
    await step();

    expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, log.length), 'R'))).toEqual([]);
    await expectEveryCrashPointSafe(log, states, 'R');
  });
});

describe('two slot writes in flight, the older one failing (review probe, recheck 2)', () => {
  const edit = (PM: any, project: any, revision: number, layer: number, name: string) => {
    project.revision = revision;
    project.layers[layer].name = name;
    PM.bus.emit('history:project-patch', { projectId: project.id, revision, patches: [{ path: ['layers', layer, 'name'], exists: true, value: name }, { path: ['revision'], exists: true, value: revision }] });
  };
  for (const newerFails of [false, true]) {
    it(`loses no edit on disk when the newer one ${newerFails ? 'fails too (the disk stays full)' : 'lands later'}`, { timeout: 120_000 }, async () => {
      const log: Op[] = [];
      let gateA: Promise<void> | null = null, gateB: Promise<void> | null = null;
      let releaseA = () => {}, releaseB = () => {};
      let renames = 0, full = false;
      const store = createStore(await temporaryDirectory(), {
        fs: loggingFs(log, {
          gate: to => {
            if (path.basename(to) !== 'project.T.json') return null;
            const n = ++renames;
            return n === 2 ? gateA : n === 3 ? gateB : null;
          },
          rename: to => path.basename(to) === 'project.T.json' && full && renames >= 2 && (renames === 2 || newerFails) ? failure('ENOSPC', 'no space left on device') : null,
        }),
        writeDelayMs: 0,
      });
      await store.load();
      const PM: any = openWindow(store);
      const project = largeProject('T');
      PM.proj = project;
      const states: string[] = [];
      const step = async (failing = false) => { await settle(PM, store, { failing }); states.push(JSON.stringify(project)); return log.length; };
      const tick = () => new Promise(resolve => setTimeout(resolve, 20));

      PM.Projects.put(project);
      await step();
      edit(PM, project, 8, 0, 'Journaled before A');
      await step();
      // Checkpoint A (its rename waits, then fails: the disk is full).
      full = true;
      gateA = new Promise<void>(resolve => { releaseA = resolve; });
      gateB = new Promise<void>(resolve => { releaseB = resolve; });
      PM.Projects.put(project);
      await tick();
      edit(PM, project, 9, 1, 'Against A');
      await tick();
      states.push(JSON.stringify(project));
      // Checkpoint B posted while A is still being written.
      PM.Projects.put(project);
      await tick();
      edit(PM, project, 10, 2, 'After B');
      await tick();
      states.push(JSON.stringify(project));
      releaseA();
      await tick(); await tick();
      const afterAFailed = log.length;
      releaseB();
      const settled = await step(true);
      if (newerFails) {
        // Still full: another edit, an autosave, a third edit.
        edit(PM, project, 11, 3, 'While full');
        await step(true);
        PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
        await step(true);
        edit(PM, project, 12, 4, 'While full 2');
        await step(true);
      }
      expect(jsonDiff(states[4]!, await recover(stateAfter(log, afterAFailed), 'T')), 'A failed, B not on disk').toEqual([]);
      expect(jsonDiff(states[4]!, await recover(stateAfter(log, settled), 'T')), 'settled').toEqual([]);
      expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, log.length), 'T')), 'last').toEqual([]);
      await expectEveryCrashPointSafe(log, states, 'T');
      full = false;
    });
  }
});

describe('a disk that stays full', () => {
  it('shows one save error per failure episode, not one per edit', { timeout: 120_000 }, async () => {
    let full = false;
    const store = createStore(await temporaryDirectory(), {
      fs: loggingFs([], { rename: () => full ? failure('ENOSPC', 'no space left on device') : null }),
      writeDelayMs: 0,
    });
    await store.load();
    const PM: any = openWindow(store);
    const project = largeProject('Z');
    PM.proj = project;
    PM.Projects.put(project);
    await settle(PM, store);
    full = true;
    for (let revision = 8; revision < 14; revision++) {
      project.layers[revision - 8].name = `Edit ${revision}`;
      PM.bus.emit('history:project-patch', { projectId: 'Z', revision, patches: [{ path: ['layers', revision - 8, 'name'], exists: true, value: `Edit ${revision}` }] });
      await settle(PM, store, { failing: true });
    }
    PM.Projects.recover(project);
    await settle(PM, store, { failing: true });
    expect(PM.toast).toHaveBeenCalledTimes(1);
    // The disk recovers; the next failure is a new episode.
    full = false;
    PM.Projects.recover(project);
    await settle(PM, store);
    full = true;
    PM.bus.emit('history:project-patch', { projectId: 'Z', revision: 20, patches: [{ path: ['name'], exists: true, value: 'Again' }] });
    await settle(PM, store, { failing: true });
    expect(PM.toast).toHaveBeenCalledTimes(2);
    full = false;
    // The disk recovers again; the next autosave's slot reopens the journal (SAVE-1b), and the
    // episode ends at the next flush without a new error: a later failure is shown.
    PM.Projects.recover(project);
    await settle(PM, store);
    await settle(PM, store);
    full = true;
    PM.Projects.recover(project);
    await settle(PM, store, { failing: true });
    expect(PM.toast).toHaveBeenCalledTimes(3);
    full = false;
  });
});

describe('a journal entry refused at the store limit (review probe, recheck 3)', () => {
  it('leaves no gap: later entries are refused until a slot holding every edit lands', { timeout: 300_000 }, async () => {
    const log: Op[] = [];
    let refuse = false;
    const store = createStore(await temporaryDirectory(), { fs: loggingFs(log), writeDelayMs: 0 });
    await store.load();
    // Slot writes refused, as for a project over the 256 MB slot limit.
    const PM: any = openWindow(store, { refuse: key => refuse && key === 'project.G' });
    const project: any = largeProject('G');
    PM.proj = project;
    const states: string[] = [];
    const step = async (failing = false) => { await settle(PM, store, { failing }); states.push(JSON.stringify(project)); return log.length; };
    const edit = (revision: number, member: string, value: unknown) => {
      project.revision = revision;
      project[member] = value;
      PM.bus.emit('history:project-patch', { projectId: 'G', revision, patches: [{ path: [member], exists: true, value }, { path: ['revision'], exists: true, value: revision }] });
    };

    PM.Projects.put(project);
    await step();
    refuse = true;
    PM.Projects.put(project);
    // The journal near its 32 MiB limit; then a 2 MiB paste (refused) and a rename made in the same moment.
    edit(8, 'blob', 'x'.repeat(31 * 1024 * 1024));
    await step(true);
    edit(9, 'paste', 'y'.repeat(2 * 1024 * 1024));
    states.push(JSON.stringify(project));
    edit(10, 'name', 'after paste');
    await step(true);
    // An autosave (its slot refused too), and another small edit.
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    edit(11, 'name', 'after autosave');
    const refused = await step(true);
    // Every crash so far recovers a state the document had (here: the one before the paste).
    expect(jsonDiff(states[1]!, await recover(stateAfter(log, refused), 'G')), 'while refused').toEqual([]);
    // The slot can be written again: it holds every edit, and the journal reopens.
    refuse = false;
    PM.Projects.recover(project, 'data:image/jpeg;base64,AAAA');
    await step();
    edit(12, 'name', 'after it saved');
    await step();
    expect(jsonDiff(states.at(-1)!, await recover(stateAfter(log, log.length), 'G')), 'after it saved').toEqual([]);
    await expect(PM.store.flush()).resolves.toBeUndefined();
    await expectEveryCrashPointSafe(log, states, 'G');
  });
});
