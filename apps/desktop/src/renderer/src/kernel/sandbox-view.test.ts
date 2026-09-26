// @vitest-environment happy-dom
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { bootRuntime, bootView, installSandboxRuntime, type SandboxView } from '../../sandbox/boot';
import type { SandboxViewInit } from '../../sandbox/shim-api';
import { createSandboxRuntime, type SandboxRuntime } from './sandbox-host';
import { mountSandboxView, type ViewHost, type ViewLink } from './sandbox-view';
import { createKernel, type Kernel } from './registries';
import { panelFrameOf } from './panel-frame';
import { ensurePanel } from '../layout/panel';
import type { HostDeps } from './host';
import type { ExtensionRecord, ProjectAPI } from './api';

let source: Buffer;
const cleanup: Array<() => void | Promise<void>> = [];
beforeAll(async () => {
  const output = await mkdtemp(path.join(os.tmpdir(), 'powermove-sandbox-view-'));
  const compilerPath = '../../../main/extensions/compiler';
  const { compileExtension } = await import(/* @vite-ignore */ compilerPath);
  const compiled = await compileExtension({ dir: path.resolve('test/fixtures/sandboxed-ext'), entry: 'index.ts', outDir: output });
  if (!compiled.ok) throw new Error(`fixture failed to compile: ${JSON.stringify(compiled)}`);
  source = await readFile(compiled.bundlePath);
  await rm(output, { recursive: true, force: true });
  installSandboxRuntime();
});
afterEach(async () => { for (const fn of cleanup.splice(0).reverse()) await fn(); document.body.replaceChildren(); vi.unstubAllGlobals(); });

const load = () => import(/* @vite-ignore */ `data:text/javascript;base64,${source.toString('base64')}`);
const tick = (ms = 20) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check: () => boolean, label: string): Promise<void> {
  for (let i = 0; i < 100; i++) { if (check()) return; await tick(10); }
  throw new Error(`timed out waiting for ${label}`);
}

interface Harness {
  kernel: Kernel; runtime: SandboxRuntime; panelId: string;
  views: Array<{ frame: HTMLIFrameElement; message: SandboxViewInit; target: HTMLElement; port: MessagePort; view: Promise<SandboxView> }>;
  pm: Record<string, any>;
}

async function start(): Promise<Harness> {
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('network mocked'))));
  const kernel = createKernel();
  const project = { get: () => ({ id: 'test' }), revision: () => 7, selection: () => ({ layers: [], keys: [], chan: null }), time: () => 0, playing: () => false, apply: vi.fn(() => ({ ok: true })), select: vi.fn(), setTime: vi.fn(), play: vi.fn(), pause: vi.fn(), undo: vi.fn(), redo: vi.fn(), snapshot: async () => '' } as unknown as ProjectAPI;
  const deps = { pm: {}, state: { doc: {}, sel: {}, transport: {}, perf: {} }, project,
    ui: { controls: {}, toast: vi.fn(), confirm: async () => true, menu: vi.fn(), modal: vi.fn(), icon: () => '' },
    assets: { pick: async () => [], import: async () => ({ id: 'x', name: 'x', kind: 'image' }), get: () => undefined, readText: async () => '' },
    storage: () => ({ get: () => undefined, set: vi.fn(), delete: vi.fn() }),
    extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: vi.fn(), rebase: vi.fn() },
    panelsBackend: { open: vi.fn(), close: vi.fn(), isOpen: () => false, refresh: vi.fn(), list: () => [] }, paletteOpen: vi.fn(), reportRuntimeError: vi.fn()
  } as unknown as HostDeps;
  const record = { id: 'sandboxed-ext', trust: 'store', scope: 'user', manifest: { id: 'sandboxed-ext', name: 'Fixture', version: '1.0.0', apiVersion: 3, permissions: ['network', 'project:write'] }, dir: '/tmp/ext', enabled: true, bundleUrl: '/ext/sandboxed-ext/bundle.js', bundleHash: 'x', health: { state: 'ok' }, updatedAt: 0 } as ExtensionRecord;
  const runtimeFrame = document.createElement('iframe');
  const views: Harness['views'] = [];
  const pending = createSandboxRuntime(kernel, record, deps, { TOKEN: 'one' }, {
    frame: runtimeFrame,
    onPostInit(port, init) { void bootRuntime(init, port, load); },
    onViewInit(frame, message, ports) {
      // The view document, played by a detached element in this realm.
      const target = document.createElement('div');
      target.dataset.viewDocument = message.panelId;
      document.body.append(target);
      views.push({ frame, message, target, port: ports[0]!, view: bootView(message, ports[0]!, ports[1]!, { load, target }) });
    }
  });
  runtimeFrame.dispatchEvent(new Event('load'));
  const runtime = await pending;
  cleanup.push(() => runtime.dispose());
  const panelId = 'sandboxed-ext.panel';
  await until(() => kernel.panels.has(panelId), 'panel registration');
  const pm: Record<string, any> = { PANELS: { [panelId]: kernel.panels.get(panelId) }, panelInst: {}, icon: () => null, Layout: { ws: null } };
  return { kernel, runtime, panelId, views, pm };
}

/* happy-dom fires `load` for an inserted iframe, as Chromium does for the
   view document; each load is a fresh connection. */
async function openPanel(h: Harness): Promise<{ element: HTMLElement; frame: HTMLIFrameElement }> {
  const element = ensurePanel(h.pm as any, { id: h.panelId } as any, { id: 'right' } as any)!;
  document.body.append(element);
  const frame = element.querySelector('iframe') as HTMLIFrameElement;
  await until(() => h.views.length === 1, 'view load');
  return { element, frame };
}

it('docks a sandboxed Svelte panel as a frame panel with host chrome, refreshes and releases it', async () => {
  const h = await start();
  const def = h.kernel.panels.get(h.panelId)!;
  expect(panelFrameOf(def)).toEqual({ extensionId: 'sandboxed-ext' });
  expect(def.header).toBeUndefined();
  expect(def.library).toBeUndefined();
  expect(def.size).toBe(220);

  const { element, frame } = await openPanel(h);
  expect(element.classList.contains('frame')).toBe(true);
  expect(element.querySelector('header .ptitle')?.textContent).toBe('Sandbox panel');
  expect(frame.getAttribute('sandbox')).toBe('allow-scripts');
  expect(frame.className).toBe('ext-panel-frame');
  expect(frame.title).toBe('Sandbox panel');

  expect(h.views).toHaveLength(1);
  const first = h.views[0]!;
  expect(first.message).toMatchObject({ t: 'init', mode: 'view', panelId: h.panelId, id: 'sandboxed-ext' });
  expect(first.message.vars).toEqual({ TOKEN: 'one' });
  await first.view;
  await until(() => frame.dataset.state === 'ready', 'view mounted');
  // The component mounted from the view's own copy of the bundle, reading the mirror.
  expect(first.target.textContent).toContain('Project revision');
  expect(first.target.querySelector('b')?.textContent).toBe('7');
  // Its registrations stayed local: the kernel still has exactly one owner per contribution.
  expect(h.kernel.commands.list().filter(command => command.id === 'sandboxed-ext.command')).toHaveLength(1);

  // Panel UI reaches the runtime's command through the kernel.
  (first.target.querySelector('button') as HTMLButtonElement).click();
  await until(() => first.target.textContent!.includes('Runs1'), 'command round trip');

  // Refresh: the layout clears the body and rebuilds; a new iframe replaces the old view.
  const inst = h.pm.panelInst[h.panelId];
  inst.body.textContent = '';
  // The test's detached view document has no browser unload event.
  (await first.view).dispose();
  inst.def.build(inst.body, inst);
  const second = inst.body.querySelector('iframe') as HTMLIFrameElement;
  expect(second).not.toBe(frame);
  expect(frame.isConnected).toBe(false);
  await until(() => first.target.childElementCount === 0, 'old view torn down');
  await until(() => h.views.length === 2, 'refreshed view load');
  await h.views[1]!.view;
  await until(() => second.dataset.state === 'ready', 'refreshed view mounted');

  // Deactivation releases the panel, the iframe, and the view's port.
  h.runtime.dispose();
  (await h.views[1]!.view).dispose();
  expect(h.kernel.panels.has(h.panelId)).toBe(false);
  expect(second.isConnected).toBe(false);
  await until(() => h.views[1]!.target.childElementCount === 0, 'view released on dispose');
});

it('ignores forged host shortcuts and dispatches only the extension’s own binding', async () => {
  const h = await start();
  const calls: string[] = [];
  const { element, frame } = await openPanel(h);
  await tick(30); // allow a queued iframe load to replace an earlier port
  const view = h.views.at(-1)!;
  await view.view;
  await until(() => frame.dataset.state === 'ready', 'view mounted');
  h.kernel.commands.register('app', { id: 'save-as', label: 'Save As', run: () => calls.push('save') });
  h.kernel.bind('app', { key: 'cmd+shift+s', command: 'save-as' });
  h.kernel.commands.register('sandboxed-ext', { id: 'sandboxed-ext.own', label: 'Own', run: () => calls.push('own') });
  h.kernel.bind('sandboxed-ext', { key: 'h', command: 'sandboxed-ext.own' });
  frame.tabIndex = 0;
  frame.focus();
  expect(document.activeElement).toBe(frame);
  const send = (key: string, metaKey = false, shiftKey = false, field = false) => view.port.postMessage({
    t: 'notify', m: 'key', a: [{ key, code: '', metaKey, ctrlKey: false, altKey: false, shiftKey, repeat: false, field }]
  });
  send('s', true, true);
  send('h', false, false, true);
  await tick(30);
  expect(calls).toEqual([]);
  send('h');
  await until(() => calls.length === 1, 'own chord');
  expect(calls).toEqual(['own']);
  expect(element.querySelector('iframe')).toBe(frame);
});

it('caps open views so their handle quotas stay within the extension budget', () => {
  const body = document.createElement('div');
  const host = { links: new Set(Array.from({ length: 50 }, () => ({} as ViewLink))) } as ViewHost;
  const mount = mountSandboxView(host, { id: 'test-panel', title: 'Test' }, body, { spec: {} });
  expect(body.textContent).toContain('open panel limit');
  expect(body.querySelector('iframe')).toBeNull();
  mount.dispose();
  expect(body.textContent).toBe('');
});
