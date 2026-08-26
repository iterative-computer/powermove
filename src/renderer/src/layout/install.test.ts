// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../legacy/registry';
import { install as installLegacyLayout } from '../legacy/ui/layout';
import { installSvelteLayout, unmountSvelteLayout } from './install';
import type { Workspace } from './model';

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
    closeMenus: vi.fn(),
    drag: vi.fn()
  };
  installLegacyLayout(PM);
  return PM;
}

function workspace(overrides: Partial<Workspace> = {}): Workspace {
  return {
    density: 'normal',
    theme: {},
    hiddenPanels: [],
    layout: { docks: [] },
    ...overrides
  };
}

let PM: PMRegistry;
let animationFrames: FrameRequestCallback[];

beforeEach(() => {
  document.body.innerHTML = '<div id="body"></div>';
  document.documentElement.removeAttribute('style');
  delete document.documentElement.dataset.density;
  animationFrames = [];
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
    animationFrames.push(callback);
    return animationFrames.length;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => undefined);
  PM = registry();
});

afterEach(async () => {
  await unmountSvelteLayout();
  vi.restoreAllMocks();
});

describe('installSvelteLayout', () => {
  it('refreshes a connected panel by clearing and rebuilding its body before updating its header', () => {
    const build = vi.fn((body: HTMLElement) => body.append(helper('span.rebuilt', 'rebuilt')));
    const header = vi.fn();
    PM.registerPanel('alpha', { title: 'Alpha', build, header });
    const current = workspace({
      layout: { docks: [{ id: 'center', flex: true, panels: [{ id: 'alpha', flex: true }] }] }
    });

    installSvelteLayout(PM);
    PM.Layout.apply(current);
    const body = document.querySelector<HTMLElement>('#panel-alpha > .body')!;
    const stray = helper('span.stray', 'stray');
    body.appendChild(stray);

    PM.Layout.refresh('alpha');

    expect(build).toHaveBeenCalledTimes(2);
    expect(build.mock.calls[1]?.[0]).toBe(body);
    expect(stray.isConnected).toBe(false);
    expect(body.querySelector('.stray')).toBeNull();
    expect(body.querySelectorAll('.rebuilt')).toHaveLength(1);
    expect(header).toHaveBeenCalledTimes(2);
  });

  it('applies theme custom properties and workspace density', () => {
    installSvelteLayout(PM);
    PM.Layout.apply(workspace({
      density: 'compact',
      theme: {
        accent: '#336699',
        bg: '#101112',
        panel: '#202122',
        text: '#f0f1f2',
        line: '#303132',
        font: 'Inter',
        mono: 'Monaco',
        radius: 8
      }
    }));

    const style = document.documentElement.style;
    expect(style.getPropertyValue('--accent')).toBe('#336699');
    expect(style.getPropertyValue('--accent-dim')).toBe('rgba(51,102,153,0.16)');
    expect(style.getPropertyValue('--accent-tx')).toBe('#336699');
    expect(style.getPropertyValue('--bg-window')).toBe('#101112');
    expect(style.getPropertyValue('--bg-panel')).toBe('#202122');
    expect(style.getPropertyValue('--tx')).toBe('#f0f1f2');
    expect(style.getPropertyValue('--line')).toBe('#303132');
    expect(style.getPropertyValue('--f-ui')).toBe('Inter');
    expect(style.getPropertyValue('--f-mono')).toBe('Monaco');
    expect([style.getPropertyValue('--r-lg'), style.getPropertyValue('--r-md'), style.getPropertyValue('--r-sm')])
      .toEqual(['8px', '5px', '3px']);
    expect(document.documentElement.dataset.density).toBe('compact');
  });

  it('emits layout synchronously and layout:applied across two animation frames', () => {
    const events: string[] = [];
    PM.bus.on('layout', () => events.push('layout'));
    PM.bus.on('layout:applied', () => events.push('layout:applied'));
    PM.invalidate.mockImplementation(() => events.push('invalidate'));
    installSvelteLayout(PM);

    PM.Layout.apply(workspace());
    expect(events).toEqual(['layout']);
    expect(animationFrames).toHaveLength(1);

    animationFrames.shift()!(16);
    expect(events).toEqual(['layout', 'layout:applied']);
    expect(animationFrames).toHaveLength(1);

    animationFrames.shift()!(32);
    expect(events).toEqual(['layout', 'layout:applied', 'layout:applied', 'invalidate']);
    expect(animationFrames).toHaveLength(0);
  });

  it('ignores a synchronous re-entrant apply requested by a layout listener', () => {
    const first = workspace({ density: 'compact' });
    const nested = workspace({ density: 'comfortable' });
    PM.bus.on('layout', () => PM.Layout.apply(nested));
    installSvelteLayout(PM);

    PM.Layout.apply(first);

    expect(PM.Layout.ws).toBe(first);
    expect(document.documentElement.dataset.density).toBe('compact');
    expect(PM.bus.emit).toHaveBeenCalledTimes(1);
    expect(animationFrames).toHaveLength(1);
  });
});
