// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { frameBus } from '../runtime/frame-bus';
import { doc } from '../state/document.svelte';
import { perf } from '../state/transport.svelte';
import StatusBar from './StatusBar.svelte';
import Titlebar from './Titlebar.svelte';
import { installShell, unmountShell } from './install';

function fakePM() {
  const listeners = new Map<string, Set<() => void>>();
  const tabs = ['p1', 'p2'];
  const projects: Record<string, any> = {
    p1: { id: 'p1', name: 'First', w: 1920, h: 1080, fps: 30, layers: [] },
    p2: { id: 'p2', name: 'Second', w: 1280, h: 720, fps: 24, layers: [] }
  };
  const bus = {
    on: vi.fn((event: string, listener: () => void) => {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      set.add(listener);
      return () => set!.delete(listener);
    }),
    emit(event: string) {
      for (const listener of [...(listeners.get(event) ?? [])]) listener();
    }
  };
  const PM: Record<string, any> = {
    ICONS: { home: '<path/>', x: '<path/>', plus: '<path/>', wand: '<path/>', grid: '<path/>', gear: '<path/>', missing: '<path/>' },
    bus,
    proj: projects.p1,
    app: { dirty: false },
    Projects: {
      tabs: vi.fn(() => [...tabs]),
      list: vi.fn(() => Object.values(projects)),
      get: vi.fn((id: string) => projects[id] ?? null),
      put: vi.fn(),
      markClosed: vi.fn((id: string) => {
        const index = tabs.indexOf(id);
        if (index >= 0) tabs.splice(index, 1);
      }),
      rename: vi.fn()
    },
    ProjectsScreen: { isOpen: false, show: vi.fn(), hide: vi.fn() },
    PANELS: {},
    GL: { gl: {} },
    allProps: (layer: any) => Object.values(layer.p ?? {}).map((prop) => ({ prop })),
    round: (value: number, precision: number) => {
      const factor = 10 ** precision;
      return Math.round(value * factor) / factor;
    },
    invalidate: vi.fn(),
    store: { get: vi.fn((_key: string, fallback: unknown) => fallback) }
  };
  return { PM, bus, projects };
}

describe('Svelte shell', () => {
  let instances: Array<ReturnType<typeof mount>>;

  beforeEach(() => {
    instances = [];
    document.body.innerHTML = '<div id="app"><div id="titlebar"><div id="tabs"></div><div class="titlebar-drag"></div><div id="tb-right"></div></div><div id="body"></div><div id="status"></div></div>';
    window.history.replaceState({}, '', '/');
    perf.fps = 0;
    perf.ms = 0;
  });

  afterEach(async () => {
    for (const instance of instances) await unmount(instance);
    unmountShell();
    frameBus.clear();
    delete (window as any).PM;
    document.body.replaceChildren();
  });

  it('renders project tabs and moves the roving tab stop with arrows, Home, and End', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const first = target.querySelector<HTMLElement>('[data-tab-id="p1"]')!;
    const second = target.querySelector<HTMLElement>('[data-tab-id="p2"]')!;
    const home = target.querySelector<HTMLElement>('[data-tab-id="home"]')!;
    const tablist = target.querySelector<HTMLElement>('[role="tablist"]')!;
    expect(tablist.getAttribute('aria-label')).toBe('Open projects');
    expect(tablist.querySelector('.project-new')).toBeNull();
    expect(target.querySelector<HTMLButtonElement>('.project-doc-close')?.tabIndex).toBe(-1);
    expect(first.tabIndex).toBe(0);
    expect(second.tabIndex).toBe(-1);

    flushSync(() => first.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })));
    expect(document.activeElement).toBe(second);
    expect(second.tabIndex).toBe(0);
    expect(first.tabIndex).toBe(-1);

    flushSync(() => second.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })));
    expect(document.activeElement).toBe(home);
    flushSync(() => home.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })));
    expect(document.activeElement).toBe(second);
  });

  it('opens the clicked project through the legacy project-open event', () => {
    const { PM, projects } = fakePM();
    PM.ProjectsScreen.isOpen = true;
    const opened = vi.fn();
    window.addEventListener('pm-open-project', opened, { once: true });
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    flushSync(() => target.querySelector<HTMLElement>('[data-tab-id="p2"]')!.click());

    expect(PM.Projects.get).toHaveBeenCalledWith('p2');
    expect(PM.ProjectsScreen.hide).toHaveBeenCalledOnce();
    expect(opened).toHaveBeenCalledOnce();
    expect((opened.mock.calls[0]![0] as CustomEvent).detail).toBe(projects.p2);
  });

  it('reacts to document ticks, perf state, and legacy dirty events with primitive shell values', () => {
    const { PM, projects } = fakePM();
    doc.replace(projects.p1);
    const status = document.getElementById('status')!;
    const titlebar = document.getElementById('titlebar')!;
    titlebar.replaceChildren();
    instances.push(mount(StatusBar, { target: status, props: { PM } }));
    instances.push(mount(Titlebar, { target: titlebar, props: { PM } }));
    flushSync();

    expect(status.textContent).toContain('0 layers');
    expect(status.textContent).toContain('0 keys');

    flushSync(() => {
      projects.p1.layers.push({ p: { opacity: { kf: [{}, {}] } } });
      doc.bumpFor('structure');
    });
    expect(status.textContent).toContain('1 layers');
    expect(status.textContent).toContain('2 keys');

    flushSync(() => {
      perf.fps = 58;
      perf.ms = 4.26;
    });
    expect(status.textContent).toContain('58 fps');
    expect(status.textContent).toContain('4.3 ms');

    flushSync(() => {
      PM.app.dirty = true;
      PM.bus.emit('project');
    });
    expect(status.textContent).toContain('UNSAVED');
    expect(titlebar.querySelector('[data-tab-id="p1"]')?.getAttribute('aria-label')).toBe('First, unsaved');
  });

  it('renames on Enter through Projects.rename and cancels on Escape', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const first = target.querySelector<HTMLElement>('[data-tab-id="p1"]')!;
    flushSync(() => first.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const committed = first.querySelector<HTMLInputElement>('.project-doc-input')!;
    expect(committed).not.toBeNull();
    flushSync(() => {
      committed.value = 'Renamed project';
      committed.dispatchEvent(new Event('input', { bubbles: true }));
      committed.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(PM.Projects.rename).toHaveBeenCalledWith('p1', 'Renamed project');
    expect(first.querySelector('.project-doc-input')).toBeNull();

    const second = target.querySelector<HTMLElement>('[data-tab-id="p2"]')!;
    flushSync(() => second.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const cancelled = second.querySelector<HTMLInputElement>('.project-doc-input')!;
    flushSync(() => {
      cancelled.value = 'Do not keep';
      cancelled.dispatchEvent(new Event('input', { bubbles: true }));
      cancelled.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(PM.Projects.rename).not.toHaveBeenCalledWith('p2', expect.anything());
    expect(second.querySelector('.project-doc-input')).toBeNull();
    expect(second.querySelector('.project-doc-label')?.textContent).toBe('Second');
  });

  it('middle-click closes a tab without hiding Projects or retaining its roving id', () => {
    const { PM, projects } = fakePM();
    PM.ProjectsScreen.isOpen = true;
    const opened = vi.fn();
    window.addEventListener('pm-open-project', opened, { once: true });
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const first = target.querySelector<HTMLElement>('[data-tab-id="p1"]')!;
    first.focus();
    flushSync(() => first.dispatchEvent(new MouseEvent('auxclick', { button: 1, bubbles: true })));

    expect(PM.Projects.markClosed).toHaveBeenCalledWith('p1');
    expect(PM.ProjectsScreen.hide).not.toHaveBeenCalled();
    expect((opened.mock.calls[0]![0] as CustomEvent).detail).toBe(projects.p2);
    expect(target.querySelector('[data-tab-id="p1"]')).toBeNull();
    expect(target.querySelector<HTMLElement>('[data-tab-id="home"]')?.tabIndex).toBe(0);
  });

  it('defaults ON and honors the shellLegacy escape hatch (Phase 5.4)', () => {
    const off = fakePM().PM;
    off.store.get = vi.fn((key: string, fallback: unknown) => (key === 'shellLegacy' ? true : fallback));
    installShell(off as any);
    expect(off.SvelteShell).toBeUndefined();
    expect(document.getElementById('titlebar')?.dataset.svelteShell).toBeUndefined();

    const stored = fakePM().PM; // fresh store → Svelte chrome by default
    installShell(stored as any);
    expect(stored.SvelteShell).toBe(true);
    expect(document.getElementById('titlebar')?.dataset.svelteShell).toBe('true');
    unmountShell();

    document.getElementById('status')!.remove();
    const missingGuard = fakePM().PM;
    missingGuard.store.get = vi.fn((key: string, fallback: unknown) => key === 'shellSvelte' ? true : fallback);
    installShell(missingGuard as any);
    expect(missingGuard.SvelteShell).toBeUndefined();

    document.getElementById('titlebar')!.replaceChildren();
    document.body.insertAdjacentHTML('beforeend', '<div id="status"></div>');
    window.history.replaceState({}, '', '/?svelte-shell');
    const queried = fakePM().PM;
    installShell(queried as any);
    expect(queried.SvelteShell).toBe(true);
  });
});
