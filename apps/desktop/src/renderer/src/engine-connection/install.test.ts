// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import type { PMRegistry } from '../legacy/registry';
import { ENGINE_VIEWER_CHANGED, ENGINE_VIEWER_SERVICE, installEngineConnection } from './install';
import type { EngineViewerService } from '../../../extensions/viewer/engine-present/presentation';

// This test exercises the connection's service eligibility, not GPU setup.
vi.mock('../../../extensions/viewer/engine-present/prewarm', () => ({
  PresentationWarmup: class { warm(): void {} dispose(): void {} }
}));

function fixture(initial: 'home' | 'project' = 'home') {
  const listeners = new Map<string, Set<() => void>>();
  const services = new Map<string, unknown>();
  const state = { project: initial, tabs: initial === 'home' ? [] as string[] : ['project'], projectsOpen: initial === 'home' };
  const bus = {
    on(name: string, listener: () => void) {
      let set = listeners.get(name);
      if (!set) { set = new Set(); listeners.set(name, set); }
      set.add(listener);
      return () => set!.delete(listener);
    },
    emit(name: string) { for (const listener of listeners.get(name) ?? []) listener(); }
  };
  const pm = {
    proj: { get id() { return state.project; }, assets: {} },
    Tabs: { get active() { return state.project === 'home' ? null : state.project; }, list: () => [...state.tabs] },
    isHomeProject: () => state.project === 'home',
    ProjectsScreen: { get isOpen() { return state.projectsOpen; } },
    bus,
    assets: { map: new Map(), get: () => null },
    Kernel: { services: {
      register(name: string, service: unknown) {
        services.set(name, service);
        return { dispose: () => { if (services.get(name) === service) services.delete(name); } };
      },
      get: (name: string) => services.get(name) ?? null
    } }
  } as unknown as PMRegistry;
  const host = {
    nativeEngine: { enabledSync: () => true, hostTime: async () => performance.now() * 1000, requestPorts() {} },
    frameBridge: {
      videoFrame: () => null, has: () => false, onSurface: () => () => {}, retire: () => 0,
      stats: () => ({ launch: 1, held: 0, received: 0, released: 0, videoFrames: 0, misses: 0, duplicates: 0, lateReleases: 0, errors: [] })
    }
  };
  return { pm, state, bus, services, host };
}

describe('engine viewer home eligibility', () => {
  it('keeps eager home and Projects overlay, suspends only after the real final tab closes, and resumes on reopen', () => {
    const f = fixture();
    let changes = 0;
    const changed = () => { changes++; };
    window.addEventListener(ENGINE_VIEWER_CHANGED, changed);
    const handle = installEngineConnection(f.pm, f.host as never, window);
    try {
      expect(handle).not.toBeNull();
      const service = f.services.get(ENGINE_VIEWER_SERVICE) as EngineViewerService;
      expect(service.mayPresent?.()).toBe(true);
      expect(changes).toBe(1); // registration only; startup home stays eager
      f.bus.emit('project');
      expect(service.mayPresent?.()).toBe(true);

      f.state.project = 'project'; f.state.tabs = ['project']; f.state.projectsOpen = false;
      f.bus.emit('project');
      f.state.projectsOpen = true;
      f.bus.emit('projects:open');
      expect(service.mayPresent?.()).toBe(true); // overlay leaves the tab open
      expect(changes).toBe(1);

      f.state.project = 'home'; f.state.tabs = [];
      f.bus.emit('project');
      expect(service.mayPresent?.()).toBe(false);
      expect(changes).toBe(2);
      f.state.project = 'project'; f.state.tabs = ['project']; f.state.projectsOpen = false;
      f.bus.emit('projects:open');
      expect(service.mayPresent?.()).toBe(true);
      expect(changes).toBe(3);

      f.state.project = 'home'; f.state.tabs = [];
      f.bus.emit('project');
      handle!.setHomeSuspendForDiagnostics(false);
      expect(service.mayPresent?.()).toBe(true); // measured control overrides close-only suspension
    } finally {
      handle?.dispose();
      window.removeEventListener(ENGINE_VIEWER_CHANGED, changed);
    }

    // A restored project can precede installation. Its final close still
    // qualifies because installation remembers the already active tab.
    const restored = fixture('project');
    const restoredHandle = installEngineConnection(restored.pm, restored.host as never, window);
    try {
      const service = restored.services.get(ENGINE_VIEWER_SERVICE) as EngineViewerService;
      expect(service.mayPresent?.()).toBe(true);
      restored.state.project = 'home'; restored.state.tabs = [];
      restored.bus.emit('project');
      expect(service.mayPresent?.()).toBe(false);
    } finally {
      restoredHandle?.dispose();
    }
  });
});
