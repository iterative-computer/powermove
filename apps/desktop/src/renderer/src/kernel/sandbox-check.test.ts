// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import type { PowermoveAPI, ProjectAPI } from './api';
import type { ExtensionPermission, ExtensionRecord } from '../../../shared/extensions';
import type { HostDeps } from './host';
import { bootRuntime, bootView } from '../../sandbox/boot';
import { createKernel } from './registries';
import { runSandboxCheck } from './sandbox-check';

afterEach(() => { document.body.replaceChildren(); vi.unstubAllGlobals(); });

function harness(activate: (api: PowermoveAPI) => void, permissions: ExtensionPermission[] = []) {
  const kernel = createKernel();
  const project = { get: () => ({ id: 'test' }), revision: () => 1,
    selection: () => ({ layers: [], keys: [], chan: null }), time: () => 0, playing: () => false,
    apply: vi.fn(() => ({ ok: true })), select: vi.fn(), setTime: vi.fn(), play: vi.fn(),
    pause: vi.fn(), undo: vi.fn(), redo: vi.fn(), snapshot: async () => '' } as unknown as ProjectAPI;
  const deps = { pm: {}, state: { doc: {}, sel: {}, transport: {}, perf: {} }, project,
    ui: { controls: {}, toast: vi.fn(), confirm: async () => true, menu: vi.fn(), modal: vi.fn(), icon: () => '' },
    assets: { pick: async () => [], import: async () => ({ id: 'x', name: 'x', kind: 'image' }), get: () => undefined, readText: async () => '' },
    storage: () => ({ get: () => undefined, set: vi.fn(), delete: vi.fn() }),
    extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: vi.fn(), rebase: vi.fn() },
    panelsBackend: { open: vi.fn(), close: vi.fn(), isOpen: () => false, refresh: vi.fn(), list: () => [] },
    paletteOpen: vi.fn() } as unknown as Omit<HostDeps, 'reportRuntimeError'>;
  const record = { id: 'check-fixture', trust: 'local', scope: 'user',
    manifest: { id: 'check-fixture', name: 'Check fixture', version: '1.0.0', apiVersion: 3, permissions },
    dir: '/tmp/check-fixture', enabled: true, bundleUrl: '/ext/check-fixture/bundle.js', bundleHash: 'x',
    health: { state: 'ok' }, updatedAt: 0 } as ExtensionRecord;
  const frame = document.createElement('iframe');
  const pending = runSandboxCheck(record, { kernel, deps, waitMs: 1000, settleMs: 0,
    runtime: {
      frame,
      onPostInit(port, init) { void bootRuntime(init, port, async () => ({ default: activate })); },
      onViewInit(_viewFrame, message, ports) {
        void bootView(message, ports[0]!, ports[1]!, {
          load: async () => ({ default: activate }), target: document.createElement('div')
        });
      }
    }
  });
  frame.dispatchEvent(new Event('load'));
  return pending;
}

it('reports a trusted-only member even when the fixture catches its error', async () => {
  const report = await harness(api => { try { void api.render.gl.bounds; } catch { /* fixture recovers */ } });
  expect(report.activation).toBe('ok');
  expect(report.permissionErrors).toEqual([{ namespace: 'render', member: 'gl', count: 1 }]);
  expect(report.ok).toBe(false);
});

it('reports a panel that throws while mounting', async () => {
  const report = await harness(api => api.panels.register({ id: 'broken', title: 'Broken', build() { throw new Error('panel boom'); } }));
  expect(report.activation).toBe('ok');
  expect(report.panels).toEqual([{ id: 'broken', mounted: false, error: 'panel boom' }]);
  expect(report.ok).toBe(false);
});

it('passes a clean fixture with a mounted panel', async () => {
  const report = await harness(api => api.panels.register({ id: 'clean', title: 'Clean', build(body) { body.textContent = 'Ready'; } }));
  expect(report.activation).toBe('ok');
  expect(report.panels).toEqual([{ id: 'clean', mounted: true }]);
  expect(report.permissionErrors).toEqual([]);
  expect(report.ok).toBe(true);
});

it('skips a full-access manifest without running the fixture', async () => {
  const activate = vi.fn();
  const report = await harness(activate, ['full-access']);
  expect(report).toMatchObject({ ok: true, skipped: 'full-access', activation: 'ok' });
  expect(activate).not.toHaveBeenCalled();
});
