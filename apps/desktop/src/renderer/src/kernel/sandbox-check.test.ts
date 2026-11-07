// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import type { PowermoveAPI, ProjectAPI } from './api';
import type { ExtensionPermission, ExtensionRecord } from '../../../shared/extensions';
import type { HostDeps } from './host';
import { bootRuntime, bootView } from '../../sandbox/boot';
import { createKernel } from './registries';
import { runSandboxCheck, quietDeps } from './sandbox-check';

afterEach(() => { document.body.replaceChildren(); vi.unstubAllGlobals(); });

it('keeps the live transport still during a compatibility check', () => {
  const step = vi.fn();
  const project = { get: () => ({}), revision: () => 1, selection: () => [], time: () => 0, playing: () => false } as unknown as ProjectAPI;
  const quiet = quietDeps({ project, transport: { step } } as unknown as Omit<HostDeps, 'reportRuntimeError'>, vi.fn());
  quiet.transport!.step(5);
  expect(step).not.toHaveBeenCalled();
});

function harness(activate: (api: PowermoveAPI) => void, permissions: ExtensionPermission[] = [], apiVersion = 3) {
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
    manifest: { id: 'check-fixture', name: 'Check fixture', version: '1.0.0', apiVersion, permissions },
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
it('reports trusted-only membership probes during the compatibility check', async () => {
  const report = await harness(api => { void ('gl' in api.render); });
  expect(report.permissionErrors).toEqual([{ namespace: 'render', member: 'gl', count: 1 }]);
  expect(report.ok).toBe(false);
});

it('passes an extension that reads the project without declaring any permission', async () => {
  let read: unknown;
  let selection: unknown;
  const report = await harness(async api => {
    read = await (api.project.get() as unknown as Promise<unknown>);
    selection = api.project.selection();
    api.events.on('project:changed', () => {});
    api.events.on('selection', () => {});
    await (api.project.snapshot() as unknown as Promise<unknown>);
  });
  expect(report.permissionErrors).toEqual([]);
  expect(report.runtimeErrors).toEqual([]);
  expect(report.ok).toBe(true);
  expect(read).toEqual({ id: 'test' });
  expect(selection).toEqual({ layers: [], keys: [], chan: null });
});

it('lets apiVersion 2 code read the project, and reports its apiVersion', async () => {
  let read: unknown;
  const report = await harness(async api => { read = await (api.project.get() as unknown as Promise<unknown>); }, [], 2);
  expect(report.apiVersion).toBe(2);
  expect(report.permissionErrors).toEqual([]);
  expect(report.ok).toBe(true);
  expect(read).toEqual({ id: 'test' });
});

it('reports a panel that throws while mounting', async () => {
  const report = await harness(api => api.panels.register({ id: 'check-fixture.broken', title: 'Broken', build() { throw new Error('panel boom'); } }));
  expect(report.activation).toBe('ok');
  expect(report.panels).toEqual([{ id: 'check-fixture.broken', mounted: false, error: 'panel boom' }]);
  expect(report.ok).toBe(false);
});

it('passes a clean fixture with a mounted panel', async () => {
  const report = await harness(api => api.panels.register({ id: 'check-fixture.clean', title: 'Clean', build(body) { body.textContent = 'Ready'; } }));
  expect(report.activation).toBe('ok');
  expect(report.panels).toEqual([{ id: 'check-fixture.clean', mounted: true }]);
  expect(report.permissionErrors).toEqual([]);
  expect(report.ok).toBe(true);
});

it('checks owned keybindings and command calls against the scratch registry', async () => {
  const report = await harness(async api => {
    api.commands.register({ id: 'check-fixture.run', label: 'Run', run: () => 'ok' });
    api.keybindings.bind({ key: 'cmd+shift+9', command: 'check-fixture.run' });
    await api.commands.run('check-fixture.run');
  });
  expect(report.ok).toBe(true);
  expect(report.activation).toBe('ok');
});

it('skips a full-access manifest without running the fixture', async () => {
  const activate = vi.fn();
  const report = await harness(activate, ['full-access']);
  expect(report).toMatchObject({ ok: true, skipped: 'full-access', activation: 'ok' });
  expect(activate).not.toHaveBeenCalled();
});

it('keeps effect scale hints as advice without failing the check', async () => {
  const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
  const report = await harness(api => {
    api.effects.register({ id: 'check-fixture-blur', label: 'Pixel Blur', group: 'Blur', params: [],
      frag: 'uniform sampler2D u_tex; uniform vec2 u_texel; varying vec2 v_uv; void main(){ gl_FragColor = texture2D(u_tex, v_uv + vec2(4.0) * u_texel); }' } as any);
    api.effects.register({ id: 'check-fixture-tint', label: 'Tint', group: 'Color', params: [],
      frag: 'uniform sampler2D u_tex; varying vec2 v_uv; void main(){ gl_FragColor = texture2D(u_tex, v_uv); }' } as any);
  });
  warn.mockRestore();
  expect(report.ok).toBe(true);
  expect(report.effectWarnings).toEqual([expect.objectContaining({ effectId: 'check-fixture.check-fixture-blur', label: 'Pixel Blur', code: 'effect-pixel-scale' })]);
});
