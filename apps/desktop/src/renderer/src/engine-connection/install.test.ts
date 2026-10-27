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

describe('engine source media', () => {
  it('announces a poster arriving as runtime media readiness, not a document write', () => {
    const f = fixture('project');
    const announced: unknown[] = [];
    const emit = f.bus.emit;
    Object.assign(f.bus, { emit: (name: string, change?: unknown) => { if (name === 'assets') announced.push(change); emit(name); } });
    const handle = installEngineConnection(f.pm, f.host as never, window);
    try {
      // A plain `assets` announcement diffs the whole document and autosaves it.
      ((f.pm as any).EngineSourceMedia.host as { changed(): void }).changed();
      expect(announced).toEqual([{ runtimeOnly: true }]);
    } finally {
      handle?.dispose();
    }
  });
});

describe('a stopped engine follows the playhead', () => {
  it('goes where the editor moved the playhead without a seek (a project opened at its saved time), and only then', () => {
    const f = fixture('project');
    const handle = installEngineConnection(f.pm, f.host as never, window)!;
    try {
      const TPS = 705_600_000;
      const pm = f.pm as unknown as { time: number; playing: boolean };
      let resting: number | null = 0.9 * TPS;
      const seeks: number[] = [];
      handle.transport.resting = () => resting;
      handle.transport.seek = (time: number) => { seeks.push(time); resting = Math.round(time * TPS); };
      // The project opened at 0 while the engine still rests at 0.9 s.
      pm.time = 0;
      pm.playing = false;
      f.bus.emit('project');
      expect(seeks).toEqual([0]);
      // In step: nothing to send.
      f.bus.emit('time');
      f.bus.emit('comps');
      expect(seeks).toEqual([0]);
      // A composition switch restoring its own time.
      pm.time = 2;
      f.bus.emit('comps');
      expect(seeks).toEqual([0, 2]);
      // Playback moves the playhead from the engine's clock.
      pm.playing = true;
      pm.time = 3;
      f.bus.emit('time');
      expect(seeks).toEqual([0, 2]);
    } finally {
      handle.dispose();
    }
  });
});
