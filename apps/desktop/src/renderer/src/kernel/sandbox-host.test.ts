// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createRpc } from '../../../shared/sandbox-rpc';
import { createSandboxAPI, PermissionError, sandboxControl } from '../../sandbox/shim-api';
import { installSandboxRuntime } from '../../sandbox/boot';
import { createSandboxRuntime, projectMirror } from './sandbox-host';
import { createKernel } from './registries';
import type { HostDeps } from './host';
import type { ExtensionRecord, ProjectAPI } from './api';
const close: Array<() => void | Promise<void>> = [];
afterEach(async () => { for (const fn of close.splice(0)) await fn(); document.body.replaceChildren(); vi.unstubAllGlobals(); });
it('registers across a real MessageChannel, caches sync callbacks, scopes vars, and disposes', async () => {
  const output = await mkdtemp(path.join(os.tmpdir(), 'powermove-sandbox-fixture-'));
  close.push(() => rm(output, { recursive: true, force: true }));
  const compilerPath = '../../../main/extensions/compiler';
  const { compileExtension } = await import(/* @vite-ignore */ compilerPath);
  const compiled = await compileExtension({ dir: path.resolve('test/fixtures/sandboxed-ext'), entry: 'index.ts', outDir: output });
  expect(compiled.ok).toBe(true);
  if (!compiled.ok) return;
  const source = await readFile(compiled.bundlePath);
  installSandboxRuntime(); // the fixture's Svelte panel resolves svelte through the runtime table
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('network mocked'))));
  const kernel = createKernel();
  const apply = vi.fn(() => ({ ok: true }));
  const project = { get: () => ({ id: 'test' }), revision: () => 1, selection: () => ({ layers: [], keys: [], chan: null }), time: () => 0, playing: () => false, apply, select: vi.fn(), setTime: vi.fn(), play: vi.fn(), pause: vi.fn(), undo: vi.fn(), redo: vi.fn(), snapshot: async () => '' } as unknown as ProjectAPI;
  const deps = { pm: {}, state: { doc: {}, sel: {}, transport: {}, perf: {} }, project,
    ui: { controls: {}, toast: vi.fn(), confirm: async () => true, menu: vi.fn(), modal: vi.fn(), icon: () => '' },
    assets: { pick: async () => [], import: async () => ({ id: 'x', name: 'x', kind: 'image' }), get: () => undefined, readText: async () => '' },
    storage: () => ({ get: () => undefined, set: vi.fn(), delete: vi.fn() }),
    extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: vi.fn(), rebase: vi.fn() },
    panelsBackend: { open: vi.fn(), close: vi.fn(), isOpen: () => false, refresh: vi.fn(), list: () => [] }, paletteOpen: vi.fn(), reportRuntimeError: vi.fn()
  } as unknown as HostDeps;
  const record = { id: 'sandboxed-ext', trust: 'store', scope: 'user', manifest: { id: 'sandboxed-ext', name: 'Fixture', version: '1.0.0', apiVersion: 3, permissions: ['network', 'project:write'] }, dir: '/tmp/ext', enabled: true, bundleUrl: '/ext/sandboxed-ext/bundle.js', bundleHash: 'x', health: { state: 'ok' }, updatedAt: 0 } as ExtensionRecord;
  const frame = document.createElement('iframe');
  let childApi!: ReturnType<typeof createSandboxAPI>;
  const runtimePromise = createSandboxRuntime(kernel, record, deps, { TOKEN: 'one' }, { frame, onPostInit(port, init) {
    const child = createRpc(port, {});
    close.push(() => child.close());
    childApi = createSandboxAPI(child, init);
    void import(`data:text/javascript;base64,${source.toString('base64')}`).then(module => {
      module.default(childApi);
      child.notify('activated');
    });
  } });
  frame.dispatchEvent(new Event('load'));
  const runtime = await runtimePromise;
  await new Promise(resolve => setTimeout(resolve, 10));
  expect(kernel.effects.topEntry('sandboxed-ext-tint')?.ownerId).toBe('sandboxed-ext');
  expect(kernel.commands.topEntry('sandboxed-ext-command')?.ownerId).toBe('sandboxed-ext');
  expect(kernel.status.topEntry('sandboxed-ext-status')?.ownerId).toBe('sandboxed-ext');
  expect(childApi.effects.get('sandboxed-ext-tint')?.label).toBe('Sandbox tint');
  expect(childApi.commands.has('sandboxed-ext-command')).toBe(true);
  expect(await childApi.commands.run('sandboxed-ext-command')).toBe('ran');
  const status = kernel.status.get('sandboxed-ext-status')!;
  expect(status.text()).toBeNull();
  await new Promise(resolve => setTimeout(resolve, 10));
  expect(status.text()).toBe('Sandbox one');
  await childApi.project.apply([]);
  expect(apply).toHaveBeenCalled();
  expect(() => childApi.host.pm).toThrow(PermissionError);
  expect(childApi.vars.keys()).toEqual(['TOKEN']);
  const cloneStart = performance.now();
  for (let i = 0; i < 100; i++) projectMirror(runtime.handle.api);
  console.info(`sandbox fixture mirror clone: ${((performance.now() - cloneStart) / 100).toFixed(3)} ms`);
  runtime.dispose();
  expect(kernel.effects.has('sandboxed-ext-tint')).toBe(false);
  expect(kernel.commands.has('sandboxed-ext-command')).toBe(false);
  expect(kernel.status.has('sandboxed-ext-status')).toBe(false);
});

it('refuses malicious port calls before they touch the host', async () => {
  const kernel = createKernel();
  const deletes = vi.fn();
  const apply = vi.fn();
  const remove = vi.fn();
  const storageSet = vi.fn();
  const hostEvent = vi.fn();
  kernel.commands.register('app', { id: 'delete', label: 'Delete', run: deletes });
  kernel.commands.register('other', { id: 'evil-ext-collide', label: 'Other', run: vi.fn() });
  kernel.events.on('project:changed', hostEvent);
  const project = { get: () => ({ id: 'test', assets: {}, library: {} }), revision: () => 1, selection: () => ({ layers: [] }), time: () => 0, playing: () => false,
    apply, select: vi.fn(), setTime: vi.fn(), play: vi.fn(), pause: vi.fn(), undo: vi.fn(), redo: vi.fn(), snapshot: async () => '' } as unknown as ProjectAPI;
  const deps = { pm: {}, state: { doc: {}, sel: {}, transport: {}, perf: {} }, project,
    ui: { controls: {}, toast: vi.fn(), confirm: async () => true, menu: vi.fn(), modal: vi.fn(), icon: () => '' },
    assets: { pick: async () => [], import: async () => ({ id: 'x', name: 'x', kind: 'image' }), get: () => undefined, readText: async () => '' },
    storage: () => ({ get: () => undefined, set: storageSet, delete: vi.fn() }),
    extensions: { list: () => [], setEnabled: vi.fn(), remove, reload: vi.fn(), reveal: vi.fn(), requestFix: vi.fn(), rebase: vi.fn(), setUp: vi.fn() },
    panelsBackend: { open: vi.fn(), close: vi.fn(), isOpen: () => false, refresh: vi.fn(), list: () => [] }, paletteOpen: vi.fn(), reportRuntimeError: vi.fn()
  } as unknown as HostDeps;
  const record = { id: 'evil-ext', trust: 'store', scope: 'user', manifest: { id: 'evil-ext', name: 'Evil', version: '1.0.0', apiVersion: 3, permissions: [] }, dir: '/tmp/evil', enabled: true, bundleUrl: '/ext/evil-ext/bundle.js', bundleHash: 'x', health: { state: 'ok' }, updatedAt: 0 } as ExtensionRecord;
  const frame = document.createElement('iframe');
  let client: ReturnType<typeof createRpc>;
  const pending = createSandboxRuntime(kernel, record, deps, {}, { frame, onPostInit(port) {
    client = createRpc(port, {});
    client.notify('activated');
  } });
  frame.dispatchEvent(new Event('load'));
  const runtime = await pending;
  close.push(() => { runtime.dispose(); client.close(); });
  const errorCode = async (promise: Promise<unknown>, code: string) => {
    await expect(promise).rejects.toMatchObject({ code });
  };
  await errorCode(client!.call('invoke', 'commands', 'run', ['delete']), 'project:write');
  await errorCode(client!.call('invoke', 'project', 'undo', []), 'project:write');
  await errorCode(client!.call('invoke', 'project', 'select', [[]]), 'project:write');
  await expect(client!.call('invoke', 'extensions', 'remove', ['other'])).rejects.toThrow('unavailable');
  await errorCode(client!.call('invoke', 'extensions', 'setUp', ['other']), 'permission_denied');
  await errorCode(client!.call('register', 'commands', 'undo', { id: 'undo', label: 'Undo', run: 1 }), 'id_collision');
  await errorCode(client!.call('register', 'commands', 'collision', { id: 'evil-ext-collide', label: 'Collision', run: 1 }), 'id_collision');
  await expect(client!.call('register', 'commands', 'malformed', { id: 'evil-ext-malformed', label: 'Malformed', run: 'callback' })).rejects.toMatchObject({ name: 'ZodError' });
  await errorCode(client!.call('invoke', 'events', 'emit', ['project:changed', {}]), 'permission_denied');
  await errorCode(client!.call('register', 'events', 'foreign-event', { event: 'other-ext:secret', fn: 1 }), 'permission_denied');
  await errorCode(client!.call('register', 'keybindings', 'bad-key', { key: 'cmd+s', command: 'delete' }), 'permission_denied');
  await errorCode(client!.call('invoke', 'storage', 'set', ['huge', 'x'.repeat(2 * 1024 * 1024)]), 'resource_limit');
  await errorCode(client!.call('invoke', 'storage', 'set', ['k'.repeat(129), 'x']), 'resource_limit');
  await client!.call('invoke', 'storage', 'set', ['first', 'x'.repeat(250 * 1024)]);
  await errorCode(client!.call('invoke', 'storage', 'set', ['second', 'x'.repeat(8 * 1024)]), 'resource_limit');
  const info = vi.spyOn(console, 'info').mockImplementation(() => {});
  for (let index = 0; index < 60; index++) client!.notify('log', 'info', 'flood', []);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(info).toHaveBeenCalledTimes(50);
  info.mockRestore();
  await new Promise(resolve => setTimeout(resolve, 1100));
  for (let index = 0; index < 200; index++) await client!.call('register', 'events', `event-${index}`, { event: 'evil-ext:tick', fn: 1 });
  await new Promise(resolve => setTimeout(resolve, 1100));
  await expect(client!.call('register', 'events', 'event-201', { event: 'evil-ext:tick', fn: 1 })).rejects.toThrow('registration limit');
  const burst = await Promise.allSettled(Array.from({ length: 500 }, () => client!.call('extensions-list')));
  expect(burst.some(result => result.status === 'rejected' && (result.reason as { code?: string }).code === 'resource_limit')).toBe(true);
  expect(deletes).not.toHaveBeenCalled();
  expect(apply).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
  expect(hostEvent).not.toHaveBeenCalled();
  expect(storageSet).toHaveBeenCalledTimes(1);
  expect(kernel.commands.topEntry('undo')).toBeUndefined();
});

it('redacts asset sources and secret-shaped project fields, and rejects an oversized mirror', () => {
  const project = { assets: { a: { blob: 'private bytes', sourcePath: '/private/file', name: 'safe' } },
    library: { token: 'secret', nested: { apiKey: 'secret', title: 'safe' } }, notes: { password: 'secret', body: 'safe' } };
  const api = { project: { get: () => project, revision: () => 7, selection: () => [], time: () => 0, playing: () => false } } as unknown as Parameters<typeof projectMirror>[0];
  const mirror = projectMirror(api);
  expect(mirror.project).toEqual({ assets: { a: { name: 'safe' } }, library: { nested: { title: 'safe' } }, notes: { body: 'safe' } });
  expect(project.library.token).toBe('secret'); // the source project was not mutated
  const huge = { project: { get: () => ({ notes: 'x'.repeat(9 * 1024 * 1024) }), revision: () => 8, selection: () => [], time: () => 0, playing: () => false } } as unknown as Parameters<typeof projectMirror>[0];
  expect(projectMirror(huge).project).toEqual({ tooLarge: true, revision: 8 });
});

it('makes an oversized mirror explicit at the shim read boundary', () => {
  const channel = new MessageChannel();
  const rpc = createRpc(channel.port1, {});
  const peer = createRpc(channel.port2, {});
  close.push(() => { rpc.close(); peer.close(); });
  const init = { id: 'evil-ext', apiVersion: 3, manifest: { id: 'evil-ext', name: 'Test', version: '1.0.0', apiVersion: 3, permissions: [] }, vars: {},
    theme: { scheme: 'dark', tokens: {} }, project: { project: { tooLarge: true, revision: 9 }, revision: 9, selection: [], time: 0, playing: false }, bundleUrl: '' } as Parameters<typeof createSandboxAPI>[1];
  const api = createSandboxAPI(rpc, init);
  expect(() => api.project.get()).toThrow('exceeds 8 MiB');
});

it('does not allocate callback handles for view-local registration replays', () => {
  const channel = new MessageChannel();
  const rpc = createRpc(channel.port1, {}, 100, { maxHandles: 1 });
  const peer = createRpc(channel.port2, { register: () => undefined });
  close.push(() => { rpc.close(); peer.close(); });
  const init = { id: 'evil-ext', apiVersion: 3, manifest: { id: 'evil-ext', name: 'Test', version: '1.0.0', apiVersion: 3, permissions: [] }, vars: {},
    theme: { scheme: 'dark', tokens: {} }, project: { project: {}, revision: 1, selection: [], time: 0, playing: false }, bundleUrl: '' } as Parameters<typeof createSandboxAPI>[1];
  const api = createSandboxAPI(rpc, init, 'view');
  api.commands.register({ id: 'evil-ext-one', label: 'One', run: () => {} });
  api.commands.register({ id: 'evil-ext-two', label: 'Two', run: () => {} });
  api.status.register({ id: 'evil-ext-status', text: () => 'ready' });
  api.palette.registerProvider(() => []);
  api.menus.contribute('panel:context', () => []);
  const events = api.events as unknown as { on(event: string, fn: () => void): { dispose(): void } };
  events.on('evil-ext:one', () => {});
  expect(() => events.on('evil-ext:two', () => {})).toThrow('handle limit');
  sandboxControl(api).dispose();
});
