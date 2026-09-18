import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './projects';

class FakeElement {
  children: any[] = [];
  className = '';
  classList = {
    values: new Set<string>(),
    add: (...names: string[]) => names.forEach(name => this.classList.values.add(name)),
    remove: (...names: string[]) => names.forEach(name => this.classList.values.delete(name)),
    contains: (name: string) => this.classList.values.has(name),
    toggle: (name: string, force?: boolean) => {
      const on = force === undefined ? !this.classList.values.has(name) : force;
      if (on) this.classList.values.add(name); else this.classList.values.delete(name);
      return on;
    },
  };
  dataset: Record<string, any> = {};
  style: Record<string, any> = {};
  textContent = '';
  value = '';
  onclick: any;
  onchange: any;
  oncontextmenu: any;
  files: any[] = [];

  constructor(public tag = 'div', public attrs: Record<string, any> = {}) {
    Object.assign(this, attrs);
  }

  append(...children: any[]) { this.children.push(...children.filter(child => child != null)); }
  appendChild(child: any) { this.children.push(child); return child; }
  addEventListener() {}
  click() { this.onclick?.({ stopPropagation() {} }); }
  querySelectorAll(selector: string): FakeElement[] {
    const all: FakeElement[] = [];
    const visit = (node: any) => {
      if (!(node instanceof FakeElement)) return;
      if (selector === '.ps-view button' && node.dataset.view) all.push(node);
      node.children.forEach(visit);
    };
    this.children.forEach(visit);
    return all;
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('legacy projects screen install', () => {
  it('connects the project library to real Open, Save, Save As, and safe duplication', async () => {
    const elements: FakeElement[] = [];
    let menuItems: any[] = [];
    const body = new FakeElement('body');
    vi.stubGlobal('document', { body });
    const dispatch = vi.fn();
    vi.stubGlobal('window', {
      CustomEvent: class { detail: any; constructor(public type: string, init: any) { this.detail = init.detail; } },
      dispatchEvent: dispatch,
      setTimeout: vi.fn(),
    });

    const h = (selector: string, ...args: any[]) => {
      const attrs = args[0] && typeof args[0] === 'object' && !(args[0] instanceof FakeElement)
        ? args.shift()
        : {};
      const el = new FakeElement(selector, attrs);
      if (attrs.dataset) Object.assign(el.dataset, attrs.dataset);
      const view = attrs['aria-label'] === 'Grid view' ? 'grid' : attrs['aria-label'] === 'List view' ? 'list' : null;
      if (view) el.dataset.view = view;
      el.append(...args);
      elements.push(el);
      return el;
    };
    const live = { id: 'P1', name: 'Hero', at: Date.now() };
    const trashed = { id: 'P2', name: 'Old', deletedAt: Date.now() };
    const saveProject = vi.fn(async () => true);
    const put = vi.fn();
    const putState = vi.fn();
    const markOpen = vi.fn();
    let PM: PMRegistry;
    const openProject = vi.fn(async () => { PM.proj = { id: 'P3' }; });
    PM = {
      h,
      icon: (name: string) => h('svg', name),
      store: { get: (_key: string, fallback: any) => fallback, set() {} },
      bus: { emit() {}, on: () => () => {} },
      Projects: {
        list: () => [live],
        trashList: () => [trashed],
        get: (id: string) => ({ id, name: id === 'P1' ? 'Hero' : 'Old', w: 1920, h: 1080, layers: [] }),
        tabs: () => ['P1'],
        getState: () => ({ time: 4, file: { path: '/tmp/Hero.pmv', savedHash: 'hash' } }),
        put,
        putState,
        markOpen,
      },
      proj: { id: 'P1' },
      projectFileState: () => ({ path: '/tmp/Hero.pmv', dirty: true }),
      menu: (_anchor: any, items: any[]) => { menuItems = items; },
      newProject() {},
      openProject,
      saveProject,
      uid: () => 'P-copy',
      toast() {},
    };

    install(PM);
    PM.ProjectsScreen.show('projects');
    const text = elements.flatMap(el => el.children).filter(value => typeof value === 'string');
    expect(elements.some(el => el.attrs.placeholder === 'Search projects')).toBe(true);
    expect(text).toContain('All Projects');
    expect(text).toContain('Recently edited');
    expect(text).toContain('Open Project…');
    expect(text.some(value => value.includes('Unsaved changes · Hero.pmv'))).toBe(true);

    const liveMore = [...elements].reverse().find(el => el.attrs['aria-label'] === 'Project actions')!;
    liveMore.onclick({ stopPropagation() {} });
    expect(menuItems.some(item => item && item.label === 'Save')).toBe(true);
    expect(menuItems.some(item => item && item.label === 'Save As…')).toBe(true);
    expect(menuItems.some(item => item && item.label === 'Move to Trash…')).toBe(true);
    await menuItems.find(item => item?.label === 'Save').run();
    await menuItems.find(item => item?.label === 'Save As…').run();
    expect(saveProject).toHaveBeenNthCalledWith(1, { projectId: 'P1', saveAs: false });
    expect(saveProject).toHaveBeenNthCalledWith(2, { projectId: 'P1', saveAs: true });

    const projectCard = elements.find(el => el.tag.startsWith('article.ps-card'))!;
    projectCard.onclick();
    expect(markOpen).toHaveBeenCalledWith('P1');
    expect(PM.ProjectsScreen.isOpen).toBe(false);

    menuItems.find(item => item?.label === 'Duplicate').run();
    expect(put).toHaveBeenCalledWith(expect.objectContaining({ id: 'P-copy', name: 'Hero copy' }), undefined);
    expect(putState).toHaveBeenCalledWith('P-copy', { time: 4 });

    const openButton = elements.find(el => el.children.includes('Open Project…'))!;
    await openButton.onclick();
    expect(openProject).toHaveBeenCalledOnce();
    expect(PM.ProjectsScreen.isOpen).toBe(false);

    PM.ProjectsScreen.show('trash');
    const trashMore = [...elements].reverse().find(el => el.attrs['aria-label'] === 'Project actions')!;
    trashMore.onclick({ stopPropagation() {} });
    expect(menuItems.some(item => item && item.label === 'Delete Forever…')).toBe(true);

    PM.ProjectsScreen.show('projects');
    PM.proj = { id: 'P1' };
    PM.Projects.tabs = () => ['P3'];
    const flushProject = vi.fn(async () => {});
    const trash = vi.fn(() => true);
    const confirm = vi.fn(async () => true);
    PM.flushProject = flushProject;
    PM.Projects.trash = trash;
    PM.confirm = confirm;
    const activeMore = [...elements].reverse().find(el => el.attrs['aria-label'] === 'Project actions')!;
    activeMore.onclick({ stopPropagation() {} });
    await menuItems.find(item => item?.label === 'Move to Trash…').run();
    expect(flushProject).toHaveBeenCalledOnce();
    expect(trash).toHaveBeenCalledWith('P1');
    expect(flushProject.mock.invocationCallOrder[0]!).toBeLessThan(trash.mock.invocationCallOrder[0]!);
    expect(dispatch.mock.calls.some(([event]) => event.type === 'pm-open-project' && (event as CustomEvent).detail.id === 'P3')).toBe(true);

    PM.proj = { id: 'P3' };
    flushProject.mockClear();
    dispatch.mockClear();
    activeMore.onclick({ stopPropagation() {} });
    await menuItems.find(item => item?.label === 'Move to Trash…').run();
    expect(trash).toHaveBeenCalledTimes(2);
    expect(flushProject).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'pm-open-project' }));

    PM.proj = { id: 'P1' };
    PM.flushProject = vi.fn(async () => { throw new Error('Disk full'); });
    activeMore.onclick({ stopPropagation() {} });
    await menuItems.find(item => item?.label === 'Move to Trash…').run();
    expect(trash).toHaveBeenCalledTimes(2);
    expect(dispatch).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'pm-open-project' }));

    let finishFlush!: () => void;
    PM.flushProject = vi.fn(() => new Promise<void>(resolve => { finishFlush = resolve; }));
    activeMore.onclick({ stopPropagation() {} });
    const pendingTrash = menuItems.find(item => item?.label === 'Move to Trash…').run();
    await Promise.resolve();
    PM.proj = { id: 'P3' };
    finishFlush();
    await pendingTrash;
    expect(trash).toHaveBeenCalledTimes(3);
    expect(dispatch).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'pm-open-project' }));
  });
});
