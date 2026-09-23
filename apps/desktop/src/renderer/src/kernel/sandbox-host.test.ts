// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createRpc } from '../../../shared/sandbox-rpc';
import { createSandboxAPI, PermissionError } from '../../sandbox/shim-api';
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
  expect(kernel.effects.topEntry('sandbox-tint')?.ownerId).toBe('sandboxed-ext');
  expect(kernel.commands.topEntry('sandbox-command')?.ownerId).toBe('sandboxed-ext');
  expect(kernel.status.topEntry('sandbox-status')?.ownerId).toBe('sandboxed-ext');
  expect(childApi.effects.get('sandbox-tint')?.label).toBe('Sandbox tint');
  expect(childApi.commands.has('sandbox-command')).toBe(true);
  expect(await childApi.commands.run('sandbox-command')).toBe('ran');
  const status = kernel.status.get('sandbox-status')!;
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
  expect(kernel.effects.has('sandbox-tint')).toBe(false);
  expect(kernel.commands.has('sandbox-command')).toBe(false);
  expect(kernel.status.has('sandbox-status')).toBe(false);
});
