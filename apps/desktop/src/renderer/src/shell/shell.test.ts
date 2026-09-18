// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { frameBus } from '../runtime/frame-bus';
import { doc } from '../state/document.svelte';
import { perf, transport } from '../state/transport.svelte';
import StatusBar from './StatusBar.svelte';
import Titlebar from './Titlebar.svelte';
import { installShell, unmountShell } from './install';

function fakePM() {
  const listeners = new Map<string, Set<() => void>>();
  const open = ['p1', 'p2'];
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
    ICONS: { home: '<path/>', x: '<path/>', plus: '<path/>', wand: '<path/>', grid: '<path/>', export: '<path/>', gear: '<path/>', missing: '<path/>' },
    bus,
    proj: projects.p1,
    app: { dirty: false },
    Projects: {
      openProjects: vi.fn(() => [...open]),
      list: vi.fn(() => Object.values(projects)),
      get: vi.fn((id: string) => projects[id] ?? null),
      put: vi.fn(),
      rename: vi.fn()
    },
    ProjectsScreen: { isOpen: false, show: vi.fn(), hide: vi.fn() },
    windows: { supported: true },
    newWindow: vi.fn(),
    menu: vi.fn(),
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
    document.body.innerHTML = '<div id="app"><div id="titlebar"><div id="doc-strip"></div><div class="titlebar-drag"></div><div id="tb-right"></div></div><div id="body"></div><div id="status"></div></div>';
    window.history.replaceState({}, '', '/');
    perf.fps = 0;
    perf.ms = 0;
    transport.playing = false;
  });

  afterEach(async () => {
    for (const instance of instances) await unmount(instance);
    unmountShell();
    frameBus.clear();
    delete (window as any).PM;
    document.body.replaceChildren();
  });

  it('does not reload document metadata on history-only changes', () => {
    const { PM, bus } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();
    PM.Projects.list.mockClear();
    PM.Projects.openProjects.mockClear();
    flushSync(() => { PM.app.dirty = true; bus.emit('history'); });
    expect(target.querySelector('[data-project-id="p1"]')?.classList.contains('dirty')).toBe(true);
    expect(PM.Projects.list).not.toHaveBeenCalled();
    expect(PM.Projects.openProjects).not.toHaveBeenCalled();
  });

  it('names only this window\'s document, whatever else is open', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const documents = target.querySelectorAll('.project-doc');
    expect(documents).toHaveLength(1);
    expect(documents[0]!.getAttribute('data-project-id')).toBe('p1');
    expect(target.querySelector('.project-doc-label')?.textContent).toBe('First');
    expect(target.querySelector('[role="tablist"]')).toBeNull();
    expect(target.querySelector('.project-doc-close')).toBeNull();
    expect(target.querySelector('.project-new')).toBeNull();
    expect(documents[0]!.classList.contains('dirty')).toBe(false);
  });

  it('updates the window document from the same mutable file state after the file bridge appears', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const tab = target.querySelector<HTMLElement>('[data-project-id="p1"]')!;
    expect(tab.title).toBe('First — Not saved to a file');
    const file = { path: '/tmp/First.pmv', dirty: false };
    PM.projectFileState = vi.fn(() => file);
    flushSync(() => PM.bus.emit('projects:open'));
    expect(target.querySelector('[data-project-id="p1"]')).toBe(tab);
    expect(tab.title).toBe('First — /tmp/First.pmv');
    expect(tab.classList.contains('dirty')).toBe(false);

    file.dirty = true;
    flushSync(() => PM.bus.emit('projects:open'));
    expect(tab.classList.contains('dirty')).toBe(true);
    expect(tab.getAttribute('aria-label')).toBe('First, unsaved');

    file.dirty = false;
    file.path = '/tmp/First-renamed.pmv';
    flushSync(() => PM.bus.emit('projects:open'));
    expect(tab.classList.contains('dirty')).toBe(false);
    expect(tab.title).toBe('First — /tmp/First-renamed.pmv');
  });

  it('goes back to the composition from Projects when the document is clicked', () => {
    const { PM } = fakePM();
    PM.ProjectsScreen.isOpen = true;
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    flushSync(() => target.querySelector<HTMLElement>('[data-project-id="p1"]')!.click());
    expect(PM.ProjectsScreen.hide).toHaveBeenCalledOnce();
  });

  it('offers rename and a new window from the document context menu', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    flushSync(() => target.querySelector<HTMLElement>('[data-project-id="p1"]')!
      .dispatchEvent(new MouseEvent('contextmenu', { bubbles: true })));
    const items = PM.menu.mock.calls[0]![1] as Array<any>;
    expect(items.filter((item) => typeof item === 'object').map((item) => item.label))
      .toEqual(['Rename project…', 'New Window']);
    items.find((item: any) => item?.label === 'New Window').run();
    expect(PM.newWindow).toHaveBeenCalledOnce();
  });

  it('opens the export dialog from the labelled titlebar button', () => {
    const { PM } = fakePM();
    PM.Export = { dialog: vi.fn() };
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const button = target.querySelector<HTMLButtonElement>('.tb-right .tb-export')!;
    expect(button.textContent?.trim()).toBe('Export');
    flushSync(() => button.click());

    expect(PM.Export.dialog).toHaveBeenCalledOnce();
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
    expect(status.textContent).toContain('Playback paused');
    expect(status.textContent).not.toContain('58 fps');
    flushSync(() => { transport.playing = true; perf.fps = 0; });
    expect(status.textContent).toContain('Measuring FPS…');
    flushSync(() => { perf.fps = 58; });
    expect(status.textContent).toContain('Preview 58 fps');
    expect(status.textContent).toContain('4.3 ms');
    flushSync(() => { transport.playing = false; });
    expect(status.textContent).not.toContain('58 fps');
    flushSync(() => { PM.Preview = { active: true }; frameBus.emit('status'); });
    expect(status.textContent).toContain('Cached preview');
    flushSync(() => { PM.Preview = { preparing: true }; frameBus.emit('status'); });
    expect(status.textContent).toContain('Preparing preview');

    flushSync(() => {
      PM.app.dirty = true;
      PM.bus.emit('project');
    });
    expect(status.textContent).toContain('UNSAVED');
    expect(titlebar.querySelector('[data-project-id="p1"]')?.getAttribute('aria-label')).toBe('First, unsaved');

    flushSync(() => {
      PM.app.dirty = false;
      PM.bus.emit('project');
    });
    expect(titlebar.querySelector('[data-project-id="p1"]')?.classList.contains('dirty')).toBe(false);
    expect(titlebar.querySelector('[data-project-id="p1"]')?.getAttribute('aria-label')).toBe('First');
  });

  it('renames on Enter through Projects.rename and cancels on Escape', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const document1 = target.querySelector<HTMLElement>('[data-project-id="p1"]')!;
    flushSync(() => document1.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const committed = document1.querySelector<HTMLInputElement>('.project-doc-input')!;
    expect(committed).not.toBeNull();
    flushSync(() => {
      committed.value = 'Renamed project';
      committed.dispatchEvent(new Event('input', { bubbles: true }));
      committed.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(PM.Projects.rename).toHaveBeenCalledWith('p1', 'Renamed project');
    expect(document1.querySelector('.project-doc-input')).toBeNull();

    flushSync(() => document1.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const cancelled = document1.querySelector<HTMLInputElement>('.project-doc-input')!;
    flushSync(() => {
      cancelled.value = 'Do not keep';
      cancelled.dispatchEvent(new Event('input', { bubbles: true }));
      cancelled.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(PM.Projects.rename).not.toHaveBeenCalledWith('p1', 'Do not keep');
    expect(document1.querySelector('.project-doc-input')).toBeNull();
    expect(document1.querySelector('.project-doc-label')?.textContent).toBe('First');
  });

  it('opens project rename with F2 and keeps the draft when saving fails', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();
    const chip = target.querySelector<HTMLElement>('[data-project-id="p1"]')!;
    flushSync(() => chip.dispatchEvent(new KeyboardEvent('keydown', { key: 'F2', bubbles: true })));
    const input = chip.querySelector<HTMLInputElement>('.project-doc-input')!;
    expect(input).not.toBeNull();
    PM.Projects.rename.mockImplementation(() => { throw new Error('Disk full'); });
    flushSync(() => {
      input.value = 'Keep this draft';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(chip.querySelector<HTMLInputElement>('.project-doc-input')?.value).toBe('Keep this draft');
  });

  it('installs unconditionally when both shell mount targets exist', () => {
    const stored = fakePM().PM;
    stored.store.get = vi.fn(() => true);
    installShell(stored as any);
    expect(document.getElementById('titlebar')?.dataset.svelteShell).toBe('true');
    expect(document.getElementById('status')?.dataset.svelteShell).toBe('true');
    expect(stored.store.get).not.toHaveBeenCalled();
    unmountShell();

    document.body.innerHTML = '<div id="app"><div id="titlebar"></div><div id="body"></div></div>';
    const missingGuard = fakePM().PM;
    installShell(missingGuard as any);
    expect(document.getElementById('titlebar')?.dataset.svelteShell).toBeUndefined();
  });
});
