// @vitest-environment happy-dom
/*
 * A panel registered with a Svelte `component` still has to satisfy the host's
 * imperative `build(body, inst)` contract, because that is what the legacy
 * layout calls. host.ts synthesises that build on top of the shared Svelte
 * runtime; these tests pin the mount, the rebuild-unmounts-first behaviour, and
 * the unmount on dispose.
 */
import type { Component } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import type { ExtensionRecord } from './api';
import { createExtensionAPI, type HostDeps } from './host';
import { installKernel } from './install';
import { createKernel, type Kernel } from './registries';
import { installRuntimeGlobals } from './runtime-globals';
import Probe from './__fixtures__/PanelProbe.svelte';

function record(id = 'notes'): ExtensionRecord {
  return {
    id,
    scope: 'user',
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1 },
    dir: `/ext/${id}`,
    enabled: true,
    bundleUrl: null,
    bundleHash: null,
    health: { state: 'ok' },
    updatedAt: 1
  };
}

function deps(): HostDeps {
  return {
    pm: {},
    ui: { toast: vi.fn(), confirm: async () => true, menu: vi.fn(), modal: vi.fn(), icon: () => '' },
    project: {} as never,
    storage: () => ({ get: () => undefined, set: vi.fn(), delete: vi.fn() }),
    extensions: {} as never,
    panelsBackend: { open: vi.fn(), close: vi.fn(), isOpen: () => false, refresh: vi.fn(), list: () => [] },
    paletteOpen: vi.fn(),
    reportRuntimeError: vi.fn()
  } as unknown as HostDeps;
}

function api(kernel: Kernel = createKernel()) {
  installRuntimeGlobals();
  return { kernel, handle: createExtensionAPI(kernel, record(), deps()) };
}

describe('component panels registered through the kernel', () => {
  it('mounts the component when the host calls build', () => {
    const { kernel, handle } = api();
    handle.api.panels.register({ id: 'notes', title: 'Notes', component: Probe as Component<never> as never });
    const body = document.createElement('div');

    kernel.panels.get('notes')!.build!(body, { spec: { note: 'hello' } });

    expect(body.querySelector('[data-probe]')?.textContent).toBe('notes:hello');
  });

  it('unmounts the previous instance on rebuild instead of stacking', () => {
    const { kernel, handle } = api();
    handle.api.panels.register({ id: 'notes', title: 'Notes', component: Probe as Component<never> as never });
    const body = document.createElement('div');
    const build = kernel.panels.get('notes')!.build!;

    build(body, { spec: { note: 'one' } });
    body.replaceChildren();
    build(body, { spec: { note: 'two' } });

    expect(body.querySelectorAll('[data-probe]')).toHaveLength(1);
    expect(body.querySelector('[data-probe]')?.textContent).toBe('notes:two');
  });

  it('unmounts on dispose', () => {
    const { kernel, handle } = api();
    handle.api.panels.register({ id: 'notes', title: 'Notes', component: Probe as Component<never> as never });
    const body = document.createElement('div');
    kernel.panels.get('notes')!.build!(body, { spec: {} });
    expect(body.querySelector('[data-probe]')).not.toBeNull();

    handle.disposeAll();

    expect(body.querySelector('[data-probe]')).toBeNull();
    expect(kernel.panels.has('notes')).toBe(false);
  });

  it('keeps an explicit build over the synthesised one', () => {
    const { kernel, handle } = api();
    const build = vi.fn();
    handle.api.panels.register({ id: 'notes', title: 'Notes', component: Probe as Component<never> as never, build });

    kernel.panels.get('notes')!.build!(document.createElement('div'), { spec: {} });

    expect(build).toHaveBeenCalledOnce();
  });
});

describe('panels backend over the legacy workspace', () => {
  function pmHarness() {
    const workspace: any = { id: 'ws' };
    const PM: any = {
      store: { get: (_key: string, fallback: unknown) => fallback, set: vi.fn() },
      bus: { on: () => () => {}, emit() {} },
      WS: { current: workspace, mutate: vi.fn((fn: any) => fn(workspace)) },
      Layout: {
        hasPanel: vi.fn(() => false),
        addPanel: vi.fn(),
        restorePanel: vi.fn(() => false),
        hidePanel: vi.fn(),
        refresh: vi.fn()
      },
      PANELS: {}
    };
    const kernel = installKernel(PM);
    return { PM, kernel, backend: kernel.deps.panelsBackend };
  }

  it('adds an absent panel to the requested dock and refreshes it', () => {
    const { PM, kernel, backend } = pmHarness();

    backend.open('notes', 'left');

    expect(PM.Layout.addPanel).toHaveBeenCalledWith({ id: 'ws' }, 'notes', 'left');
    expect(PM.Layout.refresh).toHaveBeenCalledWith('notes');
    kernel.uninstall();
  });

  it('restores a hidden panel instead of re-adding it', () => {
    const { PM, kernel, backend } = pmHarness();
    PM.Layout.restorePanel.mockReturnValue(true);

    backend.open('notes');

    expect(PM.Layout.restorePanel).toHaveBeenCalledWith({ id: 'ws' }, 'notes');
    expect(PM.Layout.addPanel).not.toHaveBeenCalled();
    kernel.uninstall();
  });

  it('leaves an already open panel where it is', () => {
    const { PM, kernel, backend } = pmHarness();
    PM.Layout.hasPanel.mockReturnValue(true);

    backend.open('notes');

    expect(PM.WS.mutate).not.toHaveBeenCalled();
    expect(PM.Layout.refresh).toHaveBeenCalledWith('notes');
    expect(backend.isOpen('notes')).toBe(true);
    kernel.uninstall();
  });

  it('hides rather than removes on close, so the spec survives', () => {
    const { PM, kernel, backend } = pmHarness();

    backend.close('notes');

    expect(PM.Layout.hidePanel).toHaveBeenCalledWith({ id: 'ws' }, 'notes');
    kernel.uninstall();
  });

  it('promotes an old workspace-local dismissal to the global panel preference', () => {
    const { PM, kernel, backend } = pmHarness();
    PM.WS.current.hiddenPanels = [{ id: 'notes' }];
    PM.Layout.rememberPanelClosed = vi.fn();
    PM.Layout.isPanelClosed = vi.fn(() => false);

    expect(backend.isHidden?.('notes')).toBe(true);
    expect(PM.Layout.rememberPanelClosed).toHaveBeenCalledWith('notes');
    kernel.uninstall();
  });

  it('lists kernel panels unioned with whatever the legacy registry still owns', () => {
    const { PM, kernel, backend } = pmHarness();
    PM.PANELS = { viewer: {} };
    kernel.panels.register('ext', { id: 'notes', title: 'Notes', build: () => {} });

    expect(backend.list().sort()).toEqual(['notes', 'viewer']);
    kernel.uninstall();
  });
});
