// @vitest-environment happy-dom
// Cross-process integration stays outside either composite TS project.
import { expect, it, vi } from 'vitest';
import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createExtensionRegistry } from '../src/main/extensions/registry';
import type { Store } from '../src/main/storage';
import { createLoader } from '../src/renderer/src/kernel/loader';
import { createKernel } from '../src/renderer/src/kernel/registries';
import { resetExtensionsStore } from '../src/renderer/src/kernel/extensions.svelte';
import type { HostDeps } from '../src/renderer/src/kernel/host';
import type { Project, ProjectAPI } from '../src/renderer/src/kernel/api';
import type { ExtensionsBridge, ExtensionsChangedEvent } from '../src/shared/extensions';
vi.mock('electron', () => ({ BrowserWindow: { getAllWindows: () => [] }, shell: {} }));

function fakeDeps(): { deps: Omit<HostDeps, 'reportRuntimeError'>; toasts: string[] } {
  const toasts: string[] = [];
  const project = {
    get: () => ({}) as Project,
    revision: () => 0,
    apply: () => ({ ok: true as const, message: 'ok', data: {} }),
    selection: () => ({ layers: [], keys: [], chan: null }),
    select: () => {},
    time: () => 0,
    setTime: () => {},
    play: () => {},
    pause: () => {},
    playing: () => false,
    undo: () => {},
    redo: () => {},
    snapshot: async () => ''
  } as unknown as ProjectAPI;
  return {
    toasts,
    deps: {
      pm: {},
      state: { doc: {}, sel: {}, transport: {}, perf: {} },
      ui: {
        controls: {} as HostDeps['ui']['controls'],
        toast: (text) => void toasts.push(text),
        confirm: async () => true,
        menu: () => {},
        modal: () => ({ close: () => {}, body: document.createElement('div') }),
        icon: () => ''
      },
      project,
      assets: { pick: async () => [], import: async (file) => ({ id: 'a', name: file.name, kind: 'model' }), get: () => undefined, readText: async () => '' },
      storage: () => ({ get: () => undefined, set: () => {}, delete: () => {} }),
      extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: () => {}, rebase: () => {} },
      panelsBackend: { open: () => {}, close: () => {}, isOpen: () => false, refresh: () => {}, list: () => [] },
      paletteOpen: () => {}
    }
  };
}

it('round-trips delayed discovery and runtime auto-disable through the authoritative registry', async () => {
  resetExtensionsStore();
  const kernel = createKernel();
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'extension-lifecycle-'));
  const userDir = path.join(root, 'extensions');
  await fs.mkdir(path.join(userDir, 'flaky'), { recursive: true });
  await fs.writeFile(path.join(userDir, 'flaky', 'manifest.json'), JSON.stringify({
    id: 'flaky', name: 'Flaky', apiVersion: 1, version: '1.0.0', entry: 'index.ts'
  }));
  await fs.writeFile(path.join(userDir, 'flaky', 'index.ts'), 'export default () => {}');
  let finishCompile!: () => void;
  const compiling = new Promise<void>(resolve => { finishCompile = resolve; });
  let onChanged: ((event: ExtensionsChangedEvent) => void) | undefined;
  const events = {
    emit: (event: ExtensionsChangedEvent) => onChanged?.(event),
    bridge: { onChanged: (listener) => { onChanged = listener; return () => { onChanged = undefined; }; } } as ExtensionsBridge
  };
  const values: Record<string, unknown> = {};
  const registry = createExtensionRegistry({ userDir, buildDir: path.join(root, 'build'),
    resourcesDir: path.join(root, 'builtin'), builtinIds: [],
    store: { snapshot: () => values, set: (key: string, value: unknown) => { values[key] = value; } } as Store,
    compile: async () => { await compiling; return { ok: true, hash: 'v1', bundlePath: 'unused' }; },
    broadcast: events.emit
  });
  events.bridge.list = async () => registry.list().map(record => ({ ...record,
    bundleUrl: 'data:text/javascript,export default api => { api.commands.register({id:"flaky.fail",label:"Fail",run:()=>{throw new Error("boom")}}); }'
  }));
  events.bridge.reportHealth = report => registry.reportHealth(report);
  const loader = createLoader({ kernel, bridge: events.bridge, deps: fakeDeps().deps, builtins: {} });
  try {
    const refresh = registry.refresh();
    await loader.boot();
    expect(loader.activeIds()).toEqual([]);
    finishCompile();
    await refresh;
    registry.emitChanged({ ids: [], reason: 'reload' });
    await loader.whenIdle();
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual(['flaky']);
    kernel.commands.get('flaky.fail')?.run();
    kernel.commands.get('flaky.fail')?.run();
    await loader.whenIdle();
    await loader.whenIdle();
    expect(registry.list()[0]).toMatchObject({ enabled: false, health: { state: 'runtime-error' } });
    expect(loader.activeIds()).toEqual([]);
    await registry.refresh();
    registry.emitChanged({ ids: ['flaky'], reason: 'reload' });
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual([]);
    await registry.setEnabled({ id: 'flaky', enabled: true });
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual(['flaky']);
  } finally {
    finishCompile();
    await loader.dispose();
    await fs.rm(root, { recursive: true, force: true });
  }
});
