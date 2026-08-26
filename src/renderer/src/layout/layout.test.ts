// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import { install as installLegacyLayout } from '../legacy/ui/layout';
import { installSvelteLayout, unmountSvelteLayout } from './install';
import type { Workspace } from './model';
import contractText from '../../../../tests-vitest/fixtures/dom-contract.json?raw';

const domContract = JSON.parse(contractText) as {
  panel: { class: string; dataAttr: string; idPrefix: string; title: string; moveHandle: string };
};

function helper(selector: string, attrs?: unknown, ...children: unknown[]): HTMLElement {
  const element = document.createElement(selector.match(/^[^.#]+/)?.[0] ?? 'div');
  const id = selector.match(/#([^.#]+)/)?.[1];
  if (id) element.id = id;
  element.classList.add(...[...selector.matchAll(/\.([^.#]+)/g)].map((match) => match[1]!));
  if (attrs instanceof Node || typeof attrs === 'string' || typeof attrs === 'number') children.unshift(attrs);
  else if (attrs && typeof attrs === 'object') {
    for (const [key, value] of Object.entries(attrs)) {
      if (key === 'style') Object.assign(element.style, value);
      else if (key.startsWith('on') && typeof value === 'function') element.addEventListener(key.slice(2), value as EventListener);
      else if (key in element) (element as any)[key] = value;
      else element.setAttribute(key, String(value));
    }
  }
  for (const child of children) if (child != null) element.append(child instanceof Node ? child : String(child));
  return element;
}

function workspace(): Workspace & Record<string, any> {
  return {
    id: 'test',
    density: 'compact',
    theme: { accent: '#336699', radius: 8 },
    hiddenPanels: [],
    layout: {
      docks: [
        { id: 'left', size: 250, panels: [{ id: 'alpha', flex: true }] },
        { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }, { id: 'beta', size: 140 }] }
      ]
    }
  };
}

function registry(): PMRegistry {
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const PM: PMRegistry = {
    h: helper,
    $: (selector: string) => document.querySelector(selector),
    $$: (selector: string) => [...document.querySelectorAll(selector)],
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    hex2rgb: (hex: string) => [
      Number.parseInt(hex.slice(1, 3), 16) / 255,
      Number.parseInt(hex.slice(3, 5), 16) / 255,
      Number.parseInt(hex.slice(5, 7), 16) / 255
    ],
    icon: (name: string) => {
      const icon = document.createElement('span');
      icon.dataset.icon = name;
      return icon;
    },
    bus: {
      emit: vi.fn((event: string, ...args: any[]) => listeners.get(event)?.forEach((listener) => listener(...args))),
      on(event: string, listener: (...args: any[]) => void) {
        let set = listeners.get(event);
        if (!set) listeners.set(event, (set = new Set()));
        set.add(listener);
        return () => set?.delete(listener);
      }
    },
    invalidate: vi.fn(),
    toast: vi.fn(),
    closeMenus: vi.fn(() => document.querySelectorAll('.drop').forEach((menu) => menu.remove())),
    drag: vi.fn()
  };
  installLegacyLayout(PM);
  const current = workspace();
  PM.WS = {
    current,
    save: vi.fn(),
    mutate: vi.fn((fn: (ws: Workspace) => void) => {
      fn(PM.WS.current);
      PM.Layout.apply(PM.WS.current);
      return PM.WS.current;
    })
  };
  return PM;
}

function register(PM: PMRegistry, id: string, options: Record<string, any> = {}): ReturnType<typeof vi.fn> {
  const build = vi.fn((body: HTMLElement) => {
    const input = document.createElement('input');
    input.value = id;
    body.appendChild(input);
  });
  PM.registerPanel(id, { title: id[0]!.toUpperCase() + id.slice(1), ...options, build });
  return build;
}

let PM: PMRegistry;
let raf = 0;

beforeEach(() => {
  document.body.innerHTML = '<div id="body"></div><div id="toasts"></div>';
  raf = 0;
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
    callback(++raf);
    return raf;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => undefined);
  PM = registry();
  (window as any).PM = PM;
});

afterEach(async () => {
  await unmountSvelteLayout();
  vi.restoreAllMocks();
  delete (window as any).PM;
});

describe('Svelte DockLayout panel pool', () => {
  it('installs unconditionally as the complete layout definition while retaining pure helper identities', () => {
    const marker = document.createElement('span');
    document.getElementById('body')!.appendChild(marker);
    const helpers = {
      resolveDropIndex: PM.Layout.resolveDropIndex,
      hitTestDockPlacement: PM.Layout.hitTestDockPlacement,
      buildDockDropTargets: PM.Layout.buildDockDropTargets,
      clampPanelHeight: PM.Layout.clampPanelHeight,
      visibleDockPlan: PM.Layout.visibleDockPlan
    };
    installSvelteLayout(PM);

    expect(marker.isConnected).toBe(false);
    for (const [name, helper] of Object.entries(helpers)) expect(PM.Layout[name]).toBe(helper);
    expect(PM.Layout).toEqual(expect.objectContaining({
      root: document.getElementById('body'),
      ws: null,
      apply: expect.any(Function),
      refresh: expect.any(Function),
      applyTheme: expect.any(Function),
      removePanel: expect.any(Function),
      hidePanel: expect.any(Function),
      restorePanel: expect.any(Function),
      addPanel: expect.any(Function),
      movePanel: expect.any(Function),
      movePanelBy: expect.any(Function),
      ensureDock: expect.any(Function),
      hasPanel: expect.any(Function),
      findPanel: expect.any(Function),
      setCollapsed: expect.any(Function)
    }));
  });

  it('builds once per id and preserves element identity while re-parenting on apply', () => {
    const builds = {
      alpha: register(PM, 'alpha'),
      viewer: register(PM, 'viewer', { headless: true, hideMoveHandle: true, flush: true, noscroll: true }),
      beta: register(PM, 'beta', { size: 140 })
    };
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const alpha = document.getElementById('panel-alpha');

    PM.WS.mutate((ws: Workspace) => PM.Layout.movePanel(ws, 'alpha', 'center'));

    expect(document.getElementById('panel-alpha')).toBe(alpha);
    expect(alpha?.closest('.dock')?.id).toBe('dock-center');
    expect(builds.alpha).toHaveBeenCalledOnce();
    expect(builds.viewer).toHaveBeenCalledOnce();
    expect(builds.beta).toHaveBeenCalledOnce();
    expect(document.querySelectorAll('#pm-panel-pool')).toHaveLength(1);
  });

  it('restores focus and text selection after a portal move', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const input = document.querySelector<HTMLInputElement>('#panel-alpha input')!;
    input.value = 'selection survives';
    input.focus();
    input.setSelectionRange(3, 12, 'backward');

    PM.WS.mutate((ws: Workspace) => PM.Layout.movePanel(ws, 'alpha', 'center'));

    expect(document.activeElement).toBe(input);
    expect([input.selectionStart, input.selectionEnd, input.selectionDirection]).toEqual([3, 12, 'backward']);
  });

  it('renders two docks, ordered slots, and legacy flex sizing', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta', { size: 140 });
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);

    expect([...document.querySelectorAll('#body > .dock')].map((dock) => dock.id)).toEqual(['dock-left', 'dock-center']);
    expect([...document.querySelectorAll('#dock-center [data-panel-slot]')].map((slot) => (slot as HTMLElement).dataset.panelSlot))
      .toEqual(['viewer', 'beta']);
    expect(document.getElementById('dock-left')?.style.flex).toBe('0 0 250px');
    expect(document.getElementById('dock-center')?.style.flex).toBe('1 1 auto');
    expect(document.getElementById('panel-viewer')?.style.flex).toBe('1 1 auto');
    expect(document.getElementById('panel-beta')?.style.flex).toBe('0 0 140px');
  });

  it('hide, restore, add, and move operations flow through WS.mutate and re-render', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    register(PM, 'gamma');
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);

    PM.WS.mutate((ws: Workspace) => PM.Layout.hidePanel(ws, 'beta'));
    expect(document.querySelector('#body > .dock #panel-beta')).toBeNull();
    expect(document.querySelector('#pm-panel-pool #panel-beta')).not.toBeNull();
    PM.WS.mutate((ws: Workspace) => PM.Layout.restorePanel(ws, 'beta'));
    expect(document.querySelector('#body > .dock #panel-beta')).not.toBeNull();
    PM.WS.mutate((ws: Workspace) => PM.Layout.addPanel(ws, 'gamma', 'left'));
    expect(document.querySelector('#dock-left #panel-gamma')).not.toBeNull();
    PM.WS.mutate((ws: Workspace) => PM.Layout.movePanelBy(ws, 'gamma', -1));
    expect(document.querySelector('#dock-left [data-panel-slot]')?.getAttribute('data-panel-slot')).toBe('gamma');
    expect(PM.WS.mutate).toHaveBeenCalledTimes(4);
  });

  it('exposes the complete stable panel selector contract', () => {
    register(PM, 'alpha', { headless: true, moveSlot: 'input', flush: true, noscroll: true });
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);

    const selector = `.${domContract.panel.class}[${domContract.panel.dataAttr}="alpha"]#${domContract.panel.idPrefix}alpha`;
    const panel = document.querySelector<HTMLElement>(selector)!;
    expect(panel).not.toBeNull();
    expect(panel.matches('.flush.noscroll.headless')).toBe(true);
    expect(panel.querySelector('header .grip')).not.toBeNull();
    expect(panel.querySelector(`header ${domContract.panel.title}`)?.textContent).toBe('Alpha');
    expect(panel.querySelector('header .sp')).not.toBeNull();
    expect(panel.querySelector('header .panel-options')).not.toBeNull();
    expect(panel.querySelector('.body')).not.toBeNull();
    expect(panel.querySelector(domContract.panel.moveHandle)).not.toBeNull();
  });

  it('opens a native-button menu with roving arrow focus and no pop-out entry', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const trigger = document.querySelector<HTMLButtonElement>('#panel-alpha .panel-options')!;
    const removeListener = vi.spyOn(window, 'removeEventListener');

    trigger.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 40, clientY: 40 }));
    const menu = document.querySelector<HTMLElement>('.drop[role="menu"]')!;
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]')];
    expect(menu.getAttribute('aria-label')).toBe('Alpha panel options');
    expect(buttons.length).toBeGreaterThan(3);
    expect(buttons.every((button) => button instanceof HTMLButtonElement)).toBe(true);
    expect(menu.textContent).not.toContain('Pop out');
    expect(buttons.some((button) => button.getAttribute('aria-disabled') === 'true')).toBe(true);
    expect(buttons.every((button) => !button.disabled)).toBe(true);
    expect(buttons.every((button) => button.style.background === '')).toBe(true);
    // Pointer-opened: focus parks on the menu, no row is painted; arrows enter the list.
    expect(document.activeElement).toBe(menu);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement).toBe(buttons[0]);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement).toBe(buttons[1]);
    expect(buttons[1]?.getAttribute('aria-disabled')).toBe('true');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(document.querySelector('.drop')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(removeListener).toHaveBeenCalledWith('keydown', expect.any(Function), true);
  });

  it('resizes and clamps a dock from the keyboard splitter', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    PM.WS.current.layout.docks[0].size = 759;
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const left = document.getElementById('dock-left')!;
    left.getBoundingClientRect = () => ({ width: 759, height: 600, left: 0, right: 759, top: 0, bottom: 600, x: 0, y: 0, toJSON() {} });
    const splitter = document.querySelector<HTMLElement>('#body > [role="separator"][aria-orientation="vertical"]')!;

    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));

    expect(PM.WS.current.layout.docks[0].size).toBe(760);
    expect(left.style.flex).toBe('0 0 760px');
    expect(splitter.getAttribute('aria-valuenow')).toBe('760');
    expect(PM.WS.save).toHaveBeenCalled();
  });

  it('resets only vertical splitters and emits only layout:applied', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    PM.WS.current.layout.docks[0].size = 430;
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const leftDock = PM.WS.current.layout.docks[0];
    const left = document.getElementById('dock-left')!;
    const vertical = document.querySelector<HTMLElement>('#body > [role="separator"][aria-orientation="vertical"]')!;
    vi.mocked(PM.bus.emit).mockClear();
    vi.mocked(PM.WS.save).mockClear();

    vertical.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));

    expect(leftDock.size).toBe(250);
    expect(leftDock.flex).toBeUndefined();
    expect(left.style.flex).toBe('0 0 250px');
    expect(vi.mocked(PM.bus.emit).mock.calls.map((call: any[]) => call[0] as string)).toEqual(['layout:applied']);
    expect(PM.WS.save).toHaveBeenCalledOnce();

    const horizontal = document.querySelector<HTMLElement>('#dock-center [role="separator"][aria-orientation="horizontal"]')!;
    const beforeSize = PM.WS.current.layout.docks[1].panels[1]?.size;
    vi.mocked(PM.bus.emit).mockClear();
    vi.mocked(PM.WS.save).mockClear();
    horizontal.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    horizontal.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(PM.WS.current.layout.docks[1].panels[1]?.size).toBe(beforeSize);
    expect(PM.bus.emit).not.toHaveBeenCalled();
    expect(PM.WS.save).not.toHaveBeenCalled();
  });

  it('computes horizontal splitter bounds from geometry and clamps arrow keys', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta', { min: 88 });
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const viewer = document.getElementById('panel-viewer')!;
    const beta = document.getElementById('panel-beta')!;
    viewer.getBoundingClientRect = () => ({ width: 500, height: 200, left: 0, right: 500, top: 0, bottom: 200, x: 0, y: 0, toJSON() {} });
    beta.getBoundingClientRect = () => ({ width: 500, height: 89, left: 0, right: 500, top: 208, bottom: 297, x: 0, y: 208, toJSON() {} });
    const splitter = document.querySelector<HTMLElement>('#dock-center [role="separator"][aria-orientation="horizontal"]')!;

    expect(splitter.hasAttribute('aria-valuemin')).toBe(false);
    expect(splitter.hasAttribute('aria-valuemax')).toBe(false);
    splitter.focus();
    flushSync();
    expect(splitter.getAttribute('aria-valuemin')).toBe('88');
    expect(splitter.getAttribute('aria-valuemax')).toBe('193');

    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));

    expect(PM.WS.current.layout.docks[1].panels[1]?.size).toBe(88);
    expect(beta.style.flex).toBe('0 0 88px');
    expect(splitter.getAttribute('aria-valuenow')).toBe('88');
  });

  it('restores both panel specs when a horizontal resize is cancelled', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta', { size: 140 });
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const viewer = document.getElementById('panel-viewer')!;
    const beta = document.getElementById('panel-beta')!;
    viewer.getBoundingClientRect = () => ({ width: 500, height: 380, left: 0, right: 500, top: 0, bottom: 380, x: 0, y: 0, toJSON() {} });
    beta.getBoundingClientRect = () => ({ width: 500, height: 140, left: 0, right: 500, top: 388, bottom: 528, x: 0, y: 388, toJSON() {} });
    const splitter = document.querySelector<HTMLElement>('#dock-center [role="separator"][aria-orientation="horizontal"]')!;
    let dragOptions: any;
    vi.mocked(PM.drag).mockImplementation(((_event: PointerEvent, options: any) => {
      dragOptions = options;
      return { cancel: vi.fn() };
    }) as any);
    vi.mocked(PM.WS.save).mockClear();

    splitter.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0, clientX: 250, clientY: 384 }) as unknown as PointerEvent);
    dragOptions.move(0, 80, new MouseEvent('pointermove', { clientX: 250, clientY: 464 }) as unknown as PointerEvent);
    expect(PM.WS.current.layout.docks[1].panels[1]).toEqual(expect.objectContaining({ id: 'beta', size: 88 }));
    expect(PM.WS.current.layout.docks[1].panels[0]).toEqual(expect.objectContaining({ id: 'viewer', flex: true }));

    dragOptions.cancel();

    expect(PM.WS.current.layout.docks[1].panels[1]).toEqual(expect.objectContaining({ id: 'beta', size: 140 }));
    expect(PM.WS.current.layout.docks[1].panels[0]).toEqual(expect.objectContaining({ id: 'viewer', flex: true }));
    expect(beta.style.flex).toBe('0 0 140px');
    expect(viewer.style.flex).toBe('1 1 auto');
    expect(PM.WS.save).not.toHaveBeenCalled();
  });

  it('lerps splitter hover glow and throttles drag layout emissions to animation frames', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta');
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const splitter = document.querySelector<HTMLElement>('#body > [role="separator"][aria-orientation="vertical"]')!;
    const left = document.getElementById('dock-left')!;
    splitter.getBoundingClientRect = () => ({ width: 8, height: 100, left: 250, right: 258, top: 0, bottom: 100, x: 250, y: 0, toJSON() {} });
    left.getBoundingClientRect = () => ({ width: 250, height: 600, left: 0, right: 250, top: 0, bottom: 600, x: 0, y: 0, toJSON() {} });
    const frames: FrameRequestCallback[] = [];
    vi.mocked(window.requestAnimationFrame).mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });
    let dragOptions: any;
    vi.mocked(PM.drag).mockImplementation(((_event: PointerEvent, options: any) => {
      dragOptions = options;
      return { cancel: vi.fn() };
    }) as any);
    const pointer = (type: string, clientY: number, button = 0): PointerEvent => {
      const event = new Event(type) as PointerEvent;
      Object.defineProperties(event, {
        clientY: { value: clientY },
        button: { value: button }
      });
      return event;
    };

    splitter.dispatchEvent(pointer('pointerenter', 0));
    splitter.dispatchEvent(pointer('pointerenter', 100));
    expect(splitter.style.getPropertyValue('--splitter-hover-y')).toBe('0.00px');
    expect(frames).toHaveLength(1);
    while (frames.length && splitter.style.getPropertyValue('--splitter-hover-y') === '0.00px') frames.shift()?.(1);
    expect(splitter.style.getPropertyValue('--splitter-hover-y')).toBe('36.00px');
    while (frames.length) frames.shift()?.(1);

    splitter.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0, clientY: 50 }) as unknown as PointerEvent);
    while (frames.length) frames.shift()?.(2);
    vi.mocked(PM.bus.emit).mockClear();
    dragOptions.move(10, 0, pointer('pointermove', 50));
    dragOptions.move(20, 0, pointer('pointermove', 50));
    expect(PM.bus.emit).not.toHaveBeenCalled();
    const cadenceFrame = frames.splice(0);
    cadenceFrame.forEach((callback) => callback(2));
    expect(vi.mocked(PM.bus.emit).mock.calls.map((call: any[]) => call[0] as string)).toEqual(['layout:applied']);
    dragOptions.up();
    expect(vi.mocked(PM.bus.emit).mock.calls.map((call: any[]) => call[0] as string)).toEqual([
      'layout:applied',
      'layout',
      'layout:applied'
    ]);
  });

  it('collapses with the legacy CSS variable and restores fixed sizing', () => {
    register(PM, 'alpha');
    register(PM, 'viewer', { headless: true, hideMoveHandle: true });
    register(PM, 'beta', { size: 140 });
    installSvelteLayout(PM);
    PM.Layout.apply(PM.WS.current);
    const panel = document.getElementById('panel-beta')!;
    const body = panel.querySelector<HTMLElement>('.body')!;

    expect(PM.Layout.setCollapsed('beta', true)).toBe(true);
    expect(panel.dataset.collapsed).toBe('1');
    expect(panel.style.flex).toBe('0 0 var(--hdr-h)');
    expect(body.style.display).toBe('none');
    expect(PM.Layout.setCollapsed('beta', false)).toBe(true);
    expect(panel.style.flex).toBe('0 0 140px');
  });
});
