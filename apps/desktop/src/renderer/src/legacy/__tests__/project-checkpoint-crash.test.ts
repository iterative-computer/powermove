// @vitest-environment happy-dom
/*
 * OPENWRITE-1 crash safety. The real window store (electron-shim), project
 * registry (core/projects) and main's store engine (main/storage, on this
 * thread) run an open's checkpoint sequence against a real directory. Every
 * mutating file operation is logged; afterwards the directory is rebuilt as it
 * stood after each prefix of that log (a crash between any two operations),
 * reloaded through a fresh store, and the project recovered the way the
 * Projects registry recovers it (slot plus journal). Every crash point must
 * recover nothing or one complete state the document actually had, never an
 * older state than an earlier crash point, and at every quiescent point
 * exactly the state today's code recovers there (today wrote the whole
 * document at every one of these requests).
 */
import * as nodeFs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { applyPatch } from '../../../../shared/patch';
import type { PMRegistry } from '../registry';
import { install as installShim } from '../host/electron-shim';
import { install as installProjects } from '../core/projects';

/* Main's store, loaded at run time: this file type-checks with the renderer,
   which does not include main's sources. */
type StoreFileSystem = any;
type Store = {
  load(): Promise<void>; get?(key: string): unknown; getEncodedSerialized?(key: string): string | null;
  bootstrapSerialized?(): Record<string, string>; set(key: string, value: unknown): void | Promise<void>;
  setSerialized?(key: string, serialized: string): void | Promise<void>; delete(key: string): void | Promise<void>; flushAll(): Promise<void>;
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

/** The file system, logging every operation that changes what a restart reads. */
function loggingFs(log: Op[]): StoreFileSystem {
  return {
    ...nodeFs,
    writeFile: (async (file: any, data: any, options: any) => {
      await nodeFs.writeFile(file, data, options);
      log.push({ kind: 'write', file: String(file), data: typeof data === 'string' ? data : Buffer.from(data).toString('utf8') });
    }) as any,
    rename: (async (from: any, to: any) => {
      await nodeFs.rename(from, to);
      log.push({ kind: 'rename', from: String(from), to: String(to) });
    }) as any,
    unlink: (async (file: any) => {
      await nodeFs.unlink(file);
      log.push({ kind: 'unlink', file: String(file) });
    }) as any,
  } as StoreFileSystem;
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

/** What a restart recovers for `id`: the slot with its journal replayed (Projects.get). */
async function recover(files: Map<string, string>, id: string): Promise<string | null> {
  const directory = await temporaryDirectory();
  for (const [name, data] of files) await nodeFs.writeFile(path.join(directory, name), data);
  const store = createStore(directory);
  await store.load();
  const slot = store.get?.(`project.${id}`) as any;
  if (!slot) return null;
  let project = structuredClone(slot.proj);
  const journal = store.get?.(`projectJournal.${id}`) as any[] | undefined;
  for (const entry of Array.isArray(journal) ? journal : []) project = applyPatch(project, entry.patches);
  await nodeFs.rm(directory, { recursive: true, force: true });
  return JSON.stringify(project);
}

/** A window whose store reaches `store` the way preload and main's IPC do. */
function openWindow(store: Store) {
  const listeners = new Map<string, Array<(payload: any) => void>>();
  const PM: PMRegistry = {
    uid: (prefix: string) => `${prefix}1`, toast: vi.fn(), version: '1.0.0',
    bus: {
      on: (event: string, listener: (payload: any) => void) => { listeners.set(event, [...listeners.get(event) ?? [], listener]); return () => {}; },
      emit: (event: string, payload?: any) => { for (const listener of listeners.get(event) ?? []) listener(payload); },
    },
    CodexBridge: { progress: vi.fn(), trace: vi.fn(), resolve: vi.fn() }, AgentArtifacts: { resolve: vi.fn() }, WindowCapture: { resolve: vi.fn() },
    store: {},
  };
  const bridge: any = {
    store: {
      bootstrapSerializedSync: () => ({ ...store.bootstrapSerialized!(), __powermoveAsyncStore: 'true' }),
      getEncodedSync: (key: string) => store.getEncodedSerialized!(key),
      setSerialized: (key: string, serialized: string) => store.setSerialized!(key, JSON.parse(JSON.stringify(serialized))),
      set: (key: string, value: unknown) => { void Promise.resolve(store.set(key, structuredClone(value))).catch(() => undefined); },
      delete: (key: string) => { void Promise.resolve(store.delete(key)).catch(() => undefined); },
      flush: () => store.flushAll(),
      onError: () => () => {},
    },
    setTheme: vi.fn(), log: vi.fn(), onMenuCommand: vi.fn(),
    codex: { run: vi.fn(), cancel: vi.fn(), requestComputerConsent: vi.fn() }, artifacts: { read: vi.fn(), reveal: vi.fn() }, captureWindow: vi.fn(), saveFile: vi.fn(),
  };
  vi.stubGlobal('window', {
    powermove: bridge, TextEncoder, structuredClone, console, setTimeout,
    atob: (value: string) => Buffer.from(value, 'base64').toString('binary'),
    btoa: (value: string) => Buffer.from(value, 'binary').toString('base64'),
    document: { readyState: 'loading', addEventListener: vi.fn(), documentElement: { classList: { add: vi.fn() } } },
    addEventListener: vi.fn(),
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

async function settle(PM: any, store: Store) {
  for (let round = 0; round < 3; round++) {
    await PM.store.flush();
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
    for (const { at, state } of settled) expect(await recover(stateAfter(log, at), 'P')).toBe(state);
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
    expect(await recover(stateAfter(log, log.length), 'S')).toBe(JSON.stringify(project));
  });
});
