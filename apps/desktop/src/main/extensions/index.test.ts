import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { IpcMain } from 'electron';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { EXT_IPC, type ExtensionRecord } from '../../shared/extensions';
import { IPC } from '../../shared/ipc';
import { extensionAssetCorsHeaders, registerExtensionsIpc, serveExtensionAsset } from './index';
import type { ExtensionRegistry } from './registry';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-ext-ipc-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) => fs.rm(directory, { recursive: true }))
  );
});

function registryStub(buildDir: string): ExtensionRegistry {
  const records: ExtensionRecord[] = [];
  return {
    buildDir,
    userDir: path.join(path.dirname(buildDir), 'user'),
    list: vi.fn(() => records),
    setEnabled: vi.fn(async () => records),
    remove: vi.fn(async () => records),
    reload: vi.fn(async () => records),
    create: vi.fn(async () => records),
    reveal: vi.fn(async () => undefined),
    readSource: vi.fn(async () => []),
    reportHealth: vi.fn(),
    refresh: vi.fn(async () => undefined),
    emitChanged: vi.fn()
  };
}

function fakeIpcMain(): {
  ipcMain: Pick<IpcMain, 'handle' | 'on'>;
  handlers: Map<string, (...args: any[]) => unknown>;
  listeners: Map<string, (...args: any[]) => unknown>;
} {
  const handlers = new Map<string, (...args: any[]) => unknown>();
  const listeners = new Map<string, (...args: any[]) => unknown>();
  const ipcMain = {
    handle: (channel: string, handler: (...args: any[]) => unknown) => handlers.set(channel, handler),
    on: (channel: string, listener: (...args: any[]) => unknown) => {
      listeners.set(channel, listener);
      return ipcMain;
    }
  };
  return {
    ipcMain: ipcMain as unknown as Pick<IpcMain, 'handle' | 'on'>,
    handlers,
    listeners
  };
}

describe('extensions IPC', () => {
  it('registers every request channel and checks sender trust', async () => {
    const buildDir = await temporaryDirectory();
    const registry = registryStub(buildDir);
    const { ipcMain, handlers, listeners } = fakeIpcMain();
    registerExtensionsIpc(ipcMain, { registry, resourcesDir: path.join(buildDir, 'builtins'), isTrusted: () => false });

    expect([...handlers.keys()].sort()).toEqual(
      [
        EXT_IPC.list,
        EXT_IPC.setEnabled,
        EXT_IPC.remove,
        EXT_IPC.reload,
        EXT_IPC.create,
        IPC.extensionFork,
        EXT_IPC.reveal,
        EXT_IPC.readSource
      ].sort()
    );
    expect(() => handlers.get(EXT_IPC.list)?.({})).toThrow('untrusted sender');
    listeners.get(EXT_IPC.reportHealth)?.({}, { id: 'valid-id', health: { state: 'ok' } });
    expect(registry.reportHealth).not.toHaveBeenCalled();
  });

  it.each([EXT_IPC.remove, EXT_IPC.reload, IPC.extensionFork, EXT_IPC.reveal, EXT_IPC.readSource])(
    'rejects traversal-shaped ids on %s',
    async (channel) => {
      const registry = registryStub(await temporaryDirectory());
      const { ipcMain, handlers } = fakeIpcMain();
      registerExtensionsIpc(ipcMain, { registry, resourcesDir: path.join(registry.buildDir, 'builtins'), isTrusted: () => true });

      await expect(Promise.resolve().then(() => handlers.get(channel)?.({}, { id: '../escape' })))
        .rejects.toThrow('expected { id }');
    }
  );

  it('validates health reports before forwarding them', async () => {
    const registry = registryStub(await temporaryDirectory());
    const { ipcMain, listeners } = fakeIpcMain();
    registerExtensionsIpc(ipcMain, { registry, resourcesDir: path.join(registry.buildDir, 'builtins'), isTrusted: () => true });
    const listener = listeners.get(EXT_IPC.reportHealth);

    listener?.({}, { id: 'valid-id', health: { state: 'build-error', error: 'no' } });
    listener?.({}, { id: 'valid-id', health: { state: 'runtime-error', error: '' } });
    listener?.({}, { id: 'valid-id', health: { state: 'runtime-error', error: 'boom' } });

    expect(registry.reportHealth).toHaveBeenCalledOnce();
    expect(registry.reportHealth).toHaveBeenCalledWith({
      id: 'valid-id',
      health: { state: 'runtime-error', error: 'boom' }
    });
  });

  it('forks a built-in into the live user directory and refreshes both records', async () => {
    const root = await temporaryDirectory();
    const buildDir = path.join(root, 'build');
    const resourcesDir = path.join(root, 'builtins');
    const registry = registryStub(buildDir);
    await fs.mkdir(path.join(resourcesDir, 'timeline'), { recursive: true });
    await fs.mkdir(registry.userDir, { recursive: true });
    await fs.writeFile(path.join(resourcesDir, 'timeline', 'manifest.json'), JSON.stringify({
      id: 'timeline', name: 'Timeline', version: '1.0.0', apiVersion: 1
    }));
    await fs.writeFile(path.join(resourcesDir, 'timeline', 'index.ts'), 'export default () => undefined');
    const { ipcMain, handlers } = fakeIpcMain();
    registerExtensionsIpc(ipcMain, { registry, resourcesDir, isTrusted: () => true });

    await expect(handlers.get(IPC.extensionFork)?.({}, { id: 'timeline' })).resolves.toEqual({ id: 'timeline-fork' });
    expect(registry.refresh).toHaveBeenCalledWith(['timeline', 'timeline-fork']);
    expect(registry.emitChanged).toHaveBeenCalledWith({ ids: ['timeline', 'timeline-fork'], reason: 'create' });
    await expect(fs.readFile(path.join(registry.userDir, 'timeline-fork', 'manifest.json'), 'utf8'))
      .resolves.toContain('"forkedFrom": "timeline@1.0.0"');
  });
});

describe('extension asset server', () => {
  it('allows only the exact Vite renderer origin during development', () => {
    expect(extensionAssetCorsHeaders('http://localhost:5174/')).toEqual({
      'Access-Control-Allow-Origin': 'http://localhost:5174'
    });
    expect(extensionAssetCorsHeaders(undefined)).toEqual({});
    expect(extensionAssetCorsHeaders('not a URL')).toEqual({});
  });

  it('serves only a generated bundle with no-store JavaScript headers', async () => {
    const buildDir = await temporaryDirectory();
    await fs.mkdir(path.join(buildDir, 'valid-id'));
    await fs.writeFile(path.join(buildDir, 'valid-id', 'bundle.js'), 'export const answer = 42;');
    const { ipcMain } = fakeIpcMain();
    registerExtensionsIpc(ipcMain, { registry: registryStub(buildDir), resourcesDir: path.join(buildDir, 'builtins'), isTrusted: () => true });

    const response = await serveExtensionAsset('/ext/valid-id/bundle.js');

    expect(response?.headers.get('content-type')).toBe('text/javascript; charset=utf-8');
    expect(response?.headers.get('cache-control')).toBe('no-store');
    await expect(response?.text()).resolves.toContain('answer = 42');
    await expect(serveExtensionAsset('/ext/valid-id/manifest.json')).resolves.toBeNull();
    await expect(serveExtensionAsset('/ext/../valid-id/bundle.js')).resolves.toBeNull();
  });

  it('rejects a bundle symlink that resolves outside the build directory', async () => {
    const buildDir = await temporaryDirectory();
    const outside = await temporaryDirectory();
    await fs.mkdir(path.join(buildDir, 'valid-id'));
    await fs.writeFile(path.join(outside, 'bundle.js'), 'malicious');
    await fs.symlink(path.join(outside, 'bundle.js'), path.join(buildDir, 'valid-id', 'bundle.js'));
    const { ipcMain } = fakeIpcMain();
    registerExtensionsIpc(ipcMain, { registry: registryStub(buildDir), resourcesDir: path.join(buildDir, 'builtins'), isTrusted: () => true });

    await expect(serveExtensionAsset('ext/valid-id/bundle.js')).resolves.toBeNull();
  });
});
