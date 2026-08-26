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
  it('installs the production library controls and project actions', () => {
    const elements: FakeElement[] = [];
    let menuItems: any[] = [];
    const body = new FakeElement('body');
    vi.stubGlobal('document', { body });

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
    const PM: PMRegistry = {
      h,
      icon: (name: string) => h('svg', name),
      store: { get: (_key: string, fallback: any) => fallback, set() {} },
      bus: { emit() {} },
      Projects: {
        list: () => [live],
        trashList: () => [trashed],
        get: (id: string) => ({ id, name: id === 'P1' ? 'Hero' : 'Old', w: 1920, h: 1080, layers: [] }),
      },
      proj: { id: 'P1' },
      menu: (_anchor: any, items: any[]) => { menuItems = items; },
      newProject() {},
      toast() {},
    };

    install(PM);
    PM.ProjectsScreen.show('projects');
    const text = elements.flatMap(el => el.children).filter(value => typeof value === 'string');
    expect(elements.some(el => el.attrs.placeholder === 'Search projects')).toBe(true);
    expect(text).toContain('All Projects');
    expect(text).toContain('Recently edited');

    const liveMore = [...elements].reverse().find(el => el.attrs['aria-label'] === 'Project actions')!;
    liveMore.onclick({ stopPropagation() {} });
    expect(menuItems.some(item => item && item.label === 'Move to Trash…')).toBe(true);

    PM.ProjectsScreen.show('trash');
    const trashMore = [...elements].reverse().find(el => el.attrs['aria-label'] === 'Project actions')!;
    trashMore.onclick({ stopPropagation() {} });
    expect(menuItems.some(item => item && item.label === 'Delete Forever…')).toBe(true);
  });
});
