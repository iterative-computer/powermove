// @vitest-environment happy-dom
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { bootRuntime, bootView, installSandboxRuntime, type SandboxView } from '../../sandbox/boot';
import type { SandboxViewInit } from '../../sandbox/shim-api';
import { createSandboxRuntime, type SandboxRuntime } from './sandbox-host';
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
  views: Array<{ frame: HTMLIFrameElement; message: SandboxViewInit; target: HTMLElement; view: Promise<SandboxView> }>;
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
      views.push({ frame, message, target, view: bootView(message, ports[0]!, ports[1]!, { load, target }) });
    }
  });
  runtimeFrame.dispatchEvent(new Event('load'));
  const runtime = await pending;
  cleanup.push(() => runtime.dispose());
  const panelId = 'sandboxed-ext-panel';
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
  expect(h.kernel.commands.list().filter(command => command.id === 'sandbox-command')).toHaveLength(1);

  // Panel UI reaches the runtime's command through the kernel.
  (first.target.querySelector('button') as HTMLButtonElement).click();
  await until(() => first.target.textContent!.includes('Runs1'), 'command round trip');

  // Refresh: the layout clears the body and rebuilds; a new iframe replaces the old view.
  const inst = h.pm.panelInst[h.panelId];
  inst.body.textContent = '';
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
  expect(h.kernel.panels.has(h.panelId)).toBe(false);
  expect(second.isConnected).toBe(false);
  await until(() => h.views[1]!.target.childElementCount === 0, 'view released on dispose');
});

it('forwards keydowns from a focused view to the host chord matcher with the frame’s field report', async () => {
  const h = await start();
  const runs: string[] = [];
  const { element, frame } = await openPanel(h);
  // The host listener sits on the panel so this test's view (same realm) cannot reach it directly.
  cleanup.push(() => listener.dispose());
  const listener = h.kernel.installKeyListener(command => { runs.push(command); return undefined; }, element);
  // Forwarded events stop at the panel, as they would at the app's own window.
  element.addEventListener('keydown', event => event.stopPropagation());
  h.kernel.bind('app', { key: 'cmd+k', command: 'palette' });
  h.kernel.bind('app', { key: 'cmd+s', command: 'save', inFields: true });
  h.kernel.bind('app', { key: 'space', command: 'play' });
  const view = h.views[0]!;
  await view.view;
  await until(() => frame.dataset.state === 'ready', 'view mounted');
  frame.tabIndex = 0;
  frame.focus();
  expect(document.activeElement).toBe(frame);
  await tick(); // the binding table reached the view

  const press = (target: EventTarget, init: KeyboardEventInit) => {
    const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(event);
    return event;
  };
  const button = view.target.querySelector('button')!;
  const input = view.target.querySelector('input')!;

  expect(press(button, { key: 'k', metaKey: true }).defaultPrevented).toBe(true);
  expect(press(button, { key: ' ', code: 'Space' }).defaultPrevented).toBe(true);
  await until(() => runs.length === 2, 'forwarded chords');
  expect(runs).toEqual(['palette', 'play']);

  // In a text field only field-aware bindings travel, and the field keeps its default.
  expect(press(input, { key: ' ', code: 'Space' }).defaultPrevented).toBe(false);
  expect(press(input, { key: 'k', metaKey: true }).defaultPrevented).toBe(false);
  expect(press(input, { key: 's', metaKey: true }).defaultPrevented).toBe(false);
  await until(() => runs.length === 3, 'field-aware chord');
  expect(runs).toEqual(['palette', 'play', 'save']);

  // Unbound keys stay in the panel; a key the panel handled itself is not forwarded.
  press(button, { key: 'j' });
  button.addEventListener('keydown', event => event.preventDefault(), { once: true });
  press(button, { key: 'k', metaKey: true });
  // Without focus the host ignores anything the frame claims.
  frame.blur();
  press(button, { key: 'k', metaKey: true });
  await tick(50);
  expect(runs).toEqual(['palette', 'play', 'save']);

  // Escape always reaches the host (menus close on it) but is not cancelled in the frame.
  const escapes: KeyboardEvent[] = [];
  element.addEventListener('keydown', event => escapes.push(event as KeyboardEvent), true);
  frame.focus();
  expect(press(button, { key: 'Escape' }).defaultPrevented).toBe(false);
  await until(() => escapes.length === 1, 'escape forwarded');
  expect(escapes[0]!.key).toBe('Escape');
});
