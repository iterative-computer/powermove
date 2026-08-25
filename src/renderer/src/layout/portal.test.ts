// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../legacy/registry';
import { install as installLegacyLayout } from '../legacy/ui/layout';
import { installSvelteLayout, unmountSvelteLayout } from './install';
import type { DockSpec, PanelSpec, Workspace } from './model';
import { panelSlot } from './portal';

function helper(selector: string, attrs?: unknown, ...children: unknown[]): HTMLElement {
  const element = document.createElement(selector.match(/^[^.#]+/)?.[0] ?? 'div');
  const id = selector.match(/#([^.#]+)/)?.[1];
  if (id) element.id = id;
  element.classList.add(...[...selector.matchAll(/\.([^.#]+)/g)].map((match) => match[1]!));
  if (attrs instanceof Node || typeof attrs === 'string' || typeof attrs === 'number') children.unshift(attrs);
  else if (attrs && typeof attrs === 'object') {
    for (const [key, value] of Object.entries(attrs)) {
      if (key === 'style') Object.assign(element.style, value);
      else if (key.startsWith('on') && typeof value === 'function') {
        element.addEventListener(key.slice(2), value as EventListener);
      } else if (key in element) (element as any)[key] = value;
      else element.setAttribute(key, String(value));
    }
  }
  for (const child of children) if (child != null) element.append(child instanceof Node ? child : String(child));
  return element;
}

function workspace(): Workspace {
  return {
    hiddenPanels: [],
    layout: {
      docks: [
        { id: 'left', size: 250, panels: [{ id: 'alpha', flex: true }] },
        { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] }
      ]
    }
  };
}

function registry(): PMRegistry {
  const PM: PMRegistry = {
    h: helper,
    $: (selector: string) => document.querySelector(selector),
    $$: (selector: string) => [...document.querySelectorAll(selector)],
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    hex2rgb: () => [0, 0, 0],
    icon: () => document.createElement('span'),
    bus: { emit: vi.fn(), on: vi.fn(() => () => undefined) },
    invalidate: vi.fn(),
    toast: vi.fn(),
    closeMenus: vi.fn(),
    drag: vi.fn(),
    Popout: { isOpen: () => false }
  };
  installLegacyLayout(PM);
  PM.SvelteShell = true;
  PM.WS = {
    current: workspace(),
    save: vi.fn(),
    mutate: vi.fn((change: (current: Workspace) => void) => {
      change(PM.WS.current);
      PM.Layout.apply(PM.WS.current);
      return PM.WS.current;
    })
  };
  return PM;
}

function registerPanels(PM: PMRegistry): void {
  PM.registerPanel('alpha', {
    title: 'Alpha',
    build(body: HTMLElement) {
      const input = document.createElement('input');
      input.setAttribute('aria-label', 'Alpha value');
      body.appendChild(input);
    }
  });
  PM.registerPanel('viewer', { title: 'Viewer', headless: true, hideMoveHandle: true, build() {} });
}

let PM: PMRegistry;

beforeEach(() => {
  document.body.innerHTML = '<div id="body"></div>';
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
    callback(0);
    return 1;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => undefined);
  PM = registry();
  registerPanels(PM);
  installSvelteLayout(PM);
  PM.Layout.apply(PM.WS.current);
});

afterEach(async () => {
  await unmountSvelteLayout();
  vi.restoreAllMocks();
});

describe('panel portal parking', () => {
  it('moves the pool to document.body and releases focus before hiding a panel', () => {
    const pool = document.getElementById('pm-panel-pool')!;
    const input = document.querySelector<HTMLInputElement>('#panel-alpha input')!;
    expect(pool.parentElement).toBe(document.body);
    input.focus();

    PM.WS.mutate((current: Workspace) => PM.Layout.hidePanel(current, 'alpha'));

    const active = document.activeElement as HTMLElement;
    expect(pool.contains(document.getElementById('panel-alpha'))).toBe(true);
    expect(pool.contains(active)).toBe(false);
    expect(active.closest('[aria-hidden="true"]')).toBeNull();
  });

  it('does not park a panel when a stale slot no longer owns its element', () => {
    const element = PM.panelInst.alpha.el as HTMLElement;
    const oldSlot = document.createElement('div');
    const currentSlot = document.createElement('div');
    document.getElementById('body')!.append(oldSlot, currentSlot);
    const spec: PanelSpec = { id: 'alpha', flex: true };
    const dock: DockSpec = { id: 'left', panels: [spec] };
    const action = panelSlot(oldSlot, { PM, id: 'alpha', spec, dock });
    currentSlot.appendChild(element);

    action.destroy();

    expect(element.parentElement).toBe(currentSlot);
  });

  it('rejoins the dock with the same panel node after removePanel then addPanel', () => {
    const element = document.getElementById('panel-alpha')!;

    PM.WS.mutate((current: Workspace) => PM.Layout.removePanel(current, 'alpha'));
    expect(document.getElementById('pm-panel-pool')?.contains(element)).toBe(true);

    PM.WS.mutate((current: Workspace) => PM.Layout.addPanel(current, 'alpha', 'left'));

    expect(document.querySelector('#body #dock-left #panel-alpha')).toBe(element);
  });
});
