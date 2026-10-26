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

  it('puts Home and the tools on the left and titles the project in the middle, without tabs', () => {
    const { PM } = fakePM();
    PM.Tabs = { list: vi.fn(() => ['p2', 'p1']), activate: vi.fn(), close: vi.fn(async () => true) };
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    expect(target.querySelector('#tabs, [role="tab"], .project-new')).toBeNull();
    const left = target.querySelector('.tb-left')!;
    expect(left.querySelector('.project-home')).not.toBeNull();
    expect(left.querySelector('#toolbar-strip')).not.toBeNull();
    expect(target.querySelector('.tb-right #toolbar-strip')).toBeNull();
    const title = target.querySelector<HTMLButtonElement>('#doc-title .tb-title-btn')!;
    expect(title.dataset.projectId).toBe('p1');
    expect(title.querySelector('.tb-title-name')?.textContent).toBe('First');
    expect(title.getAttribute('aria-haspopup')).toBe('menu');
  });

  it('switches, renames and closes open projects from the title menu', () => {
    const { PM } = fakePM();
    PM.Tabs = { list: vi.fn(() => ['p2', 'p1']), activate: vi.fn(), close: vi.fn(async () => true), closeOthers: vi.fn(), moveToNewWindow: vi.fn() };
    PM.newProject = vi.fn();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    flushSync(() => target.querySelector<HTMLButtonElement>('.tb-title-btn')!.click());
    const items = PM.menu.mock.calls.at(-1)![1] as Array<any>;
    expect(items.filter((item) => typeof item === 'object').map((item) => item.label)).toEqual([
      'Second', 'First', 'Rename Project…', 'Close Project', 'Close Other Projects', 'Move to New Window', 'New Project', 'All Projects…'
    ]);
    // The project on screen is ticked; the other is a click away.
    expect(items.find((item: any) => item?.label === 'First').on).toBe(true);
    items.find((item: any) => item?.label === 'Second').run();
    expect(PM.Tabs.activate).toHaveBeenCalledWith('p2');
    items.find((item: any) => item?.label === 'Close Project').run();
    expect(PM.Tabs.close).toHaveBeenCalledWith('p1');
    items.find((item: any) => item?.label === 'Close Other Projects').run();
    expect(PM.Tabs.closeOthers).toHaveBeenCalledWith('p1');
    items.find((item: any) => item?.label === 'New Project').run();
    expect(PM.newProject).toHaveBeenCalledOnce();
    items.find((item: any) => item?.label === 'All Projects…').run();
    expect(PM.ProjectsScreen.show).toHaveBeenCalledOnce();
  });

  it('has nothing to close or move when one project is open', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    flushSync(() => target.querySelector<HTMLElement>('.tb-title-btn')!
      .dispatchEvent(new MouseEvent('contextmenu', { bubbles: true })));
    const items = PM.menu.mock.calls.at(-1)![1] as Array<any>;
    expect(items.find((item: any) => item?.label === 'Close Other Projects').disabled).toBe(true);
    expect(items.find((item: any) => item?.label === 'Move to New Window').disabled).toBe(true);
  });

  it('shows no title while the window has only the placeholder composition', () => {
    const { PM } = fakePM();
    PM.Tabs = { list: vi.fn(() => []) };
    PM.ProjectsScreen.isOpen = true;
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    expect(target.querySelector('.tb-title-btn')).toBeNull();
    expect(target.querySelector('.project-home')?.getAttribute('aria-pressed')).toBe('true');
    // Export has nothing to export from Projects.
    expect(target.querySelector('.tb-export')).toBeNull();
  });

  it('titles a project that is still opening as busy until it lands', () => {
    const { PM, bus } = fakePM();
    PM.Tabs = { list: vi.fn(() => ['p1', 'p2']), activate: vi.fn() };
    PM.projectFileState = vi.fn(() => ({ path: '/tmp/x.pmv', dirty: true }));
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    PM.projectFileState.mockClear();
    flushSync(() => {
      PM.projectLoading = { id: 'p2', name: 'Second', generation: 1 };
      bus.emit('project:loading');
    });
    const title = target.querySelector<HTMLElement>('.tb-title-btn')!;
    expect(title.dataset.projectId).toBe('p2');
    expect(title.getAttribute('aria-busy')).toBe('true');
    expect(title.classList.contains('pending')).toBe(true);
    expect(title.getAttribute('aria-label')).toBe('Second, opening');
    // The opening project never reads the saved state the load is reading.
    expect(PM.projectFileState).not.toHaveBeenCalledWith('p2', expect.anything());

    flushSync(() => {
      PM.proj = { id: 'p2', name: 'Second' };
      PM.projectLoading = null;
      bus.emit('project:loading');
    });
    expect(target.querySelector('.tb-title-btn')?.hasAttribute('aria-busy')).toBe(false);
    expect(target.querySelector('.tb-title-btn')?.classList.contains('pending')).toBe(false);
  });

  it('titles a project opening on a cold boot', () => {
    const { PM, bus } = fakePM();
    PM.Tabs = { list: vi.fn(() => []), activate: vi.fn() };
    PM.proj = undefined;
    PM.projectLoading = { id: 'p9', name: 'Long Film', generation: 1 };
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    expect(target.querySelector('.tb-title-name')?.textContent).toBe('Long Film');
    expect(target.querySelector('.tb-title-btn')?.getAttribute('aria-busy')).toBe('true');

    flushSync(() => {
      PM.Tabs.list = vi.fn(() => ['p9']);
      PM.proj = { id: 'p9', name: 'Long Film' };
      PM.projectLoading = null;
      bus.emit('project:loading');
    });
    expect(target.querySelector('.tb-title-btn')?.hasAttribute('aria-busy')).toBe(false);
    expect((target.querySelector('.tb-title-btn') as HTMLElement).dataset.projectId).toBe('p9');
  });

  it('updates the window document from the same mutable file state after the file bridge appears', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    const title = () => target.querySelector<HTMLElement>('[data-project-id="p1"]')!;
    expect(title().title).toBe('First — Not saved to a file');
    const file = { path: '/tmp/First.pmv', dirty: false };
    PM.projectFileState = vi.fn(() => file);
    flushSync(() => PM.bus.emit('projects:open'));
    expect(title().title).toBe('First — /tmp/First.pmv');
    expect(title().classList.contains('dirty')).toBe(false);

    expect(title().querySelector('.tb-title-state')).toBeNull();
    file.dirty = true;
    flushSync(() => PM.bus.emit('projects:open'));
    // Unsaved reads as a word after the name, the way a macOS window titles itself.
    expect(title().querySelector('.tb-title-state')?.textContent).toBe('Edited');
    expect(title().getAttribute('aria-label')).toBe('First, unsaved');

    file.dirty = false;
    file.path = '/tmp/First-renamed.pmv';
    flushSync(() => PM.bus.emit('projects:open'));
    expect(title().classList.contains('dirty')).toBe(false);
    expect(title().title).toBe('First — /tmp/First-renamed.pmv');
  });

  it('goes back to the composition from Projects when the title is clicked', () => {
    const { PM } = fakePM();
    PM.ProjectsScreen.isOpen = true;
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();

    flushSync(() => target.querySelector<HTMLElement>('[data-project-id="p1"]')!.click());
    expect(PM.ProjectsScreen.hide).toHaveBeenCalledOnce();
    expect(PM.menu).not.toHaveBeenCalled();
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
    // The frame time is in the playback field's tooltip, not the bar.
    expect(status.querySelector('[title*="4.3 ms"]')).not.toBeNull();
    expect(status.textContent).not.toContain('WebGL2');
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

    const title = () => target.querySelector<HTMLElement>('[data-project-id="p1"]')!;
    const rename = () => {
      flushSync(() => title().click());
      flushSync(() => (PM.menu.mock.calls.at(-1)![1] as Array<any>).find((item) => item?.label === 'Rename Project…').run());
    };
    rename();
    const committed = target.querySelector<HTMLInputElement>('.tb-title-input')!;
    expect(committed).not.toBeNull();
    flushSync(() => {
      committed.value = 'Renamed project';
      committed.dispatchEvent(new Event('input', { bubbles: true }));
      committed.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(PM.Projects.rename).toHaveBeenCalledWith('p1', 'Renamed project');
    expect(target.querySelector('.tb-title-input')).toBeNull();

    rename();
    const cancelled = target.querySelector<HTMLInputElement>('.tb-title-input')!;
    flushSync(() => {
      cancelled.value = 'Do not keep';
      cancelled.dispatchEvent(new Event('input', { bubbles: true }));
      cancelled.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(PM.Projects.rename).not.toHaveBeenCalledWith('p1', 'Do not keep');
    expect(target.querySelector('.tb-title-input')).toBeNull();
    expect(title().querySelector('.tb-title-name')?.textContent).toBe('First');
  });

  it('opens project rename with F2 and keeps the draft when saving fails', () => {
    const { PM } = fakePM();
    const target = document.getElementById('titlebar')!;
    target.replaceChildren();
    instances.push(mount(Titlebar, { target, props: { PM } }));
    flushSync();
    const title = target.querySelector<HTMLElement>('[data-project-id="p1"]')!;
    flushSync(() => title.dispatchEvent(new KeyboardEvent('keydown', { key: 'F2', bubbles: true })));
    const input = target.querySelector<HTMLInputElement>('.tb-title-input')!;
    expect(input).not.toBeNull();
    PM.Projects.rename.mockImplementation(() => { throw new Error('Disk full'); });
    flushSync(() => {
      input.value = 'Keep this draft';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(target.querySelector<HTMLInputElement>('.tb-title-input')?.value).toBe('Keep this draft');
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
