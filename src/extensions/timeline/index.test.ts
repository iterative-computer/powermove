// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI } from 'powermove';

import activate from './index';

function domHelper(selector: string, attrs?: unknown, ...children: unknown[]): HTMLElement {
  const element = document.createElement(selector.match(/^[^.#]+/)?.[0] ?? 'div');
  const id = selector.match(/#([^.#]+)/)?.[1];
  if (id) element.id = id;
  element.classList.add(...[...selector.matchAll(/\.([^.#]+)/g)].map((match) => match[1]!));
  if (attrs instanceof Node || typeof attrs === 'string' || typeof attrs === 'number') children.unshift(attrs);
  else if (attrs && typeof attrs === 'object') {
    for (const [key, value] of Object.entries(attrs)) {
      if (key === 'style') Object.assign(element.style, value);
      else if (key in element) (element as any)[key] = value;
      else element.setAttribute(key, String(value));
    }
  }
  for (const child of children) {
    if (child != null) element.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return element;
}

function timelinePM(): Record<string, any> {
  const busReleases: Array<ReturnType<typeof vi.fn>> = [];
  return {
    h: domHelper,
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    bus: { on: vi.fn(() => {
      const release = vi.fn();
      busReleases.push(release);
      return release;
    }) },
    __timelineBusReleases: busReleases,
    proj: { w: 1920, h: 1080, fps: 30, dur: 10, work: [0, 10], layers: [], markers: [] },
    $: (selector: string) => document.querySelector(selector),
    icon: (name: string) => {
      const icon = document.createElement('span');
      icon.dataset.icon = name;
      return icon;
    },
    snap: true,
    loop: false,
    time: 0,
    playing: false,
    tc: () => '00:00:00:00',
    toggle: vi.fn(),
    setTime: vi.fn(),
    invalidate: vi.fn()
  };
}

describe('timeline extension', () => {
  beforeEach(() => {
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({} as CanvasRenderingContext2D));
    vi.stubGlobal('ResizeObserver', class {
      observe(): void {}
      disconnect(): void {}
      unobserve(): void {}
    });
  });

  afterEach(() => {
    document.body.replaceChildren();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('registers the timeline panel with the legacy layout contract', () => {
    let panel: PanelDefinition | undefined;
    const PM = {
      h: vi.fn(),
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      bus: { on: vi.fn() },
      invalidate: vi.fn()
    };
    const api = {
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI;

    activate(api);

    expect(panel).toMatchObject({
      id: 'timeline',
      title: 'Timeline',
      flush: true,
      noscroll: true,
      headless: true,
      size: 340,
      moveSlot: '#tl-head'
    });
    expect(panel?.build).toEqual(expect.any(Function));
    expect((PM as Record<string, any>).TL).toBeDefined();
  });

  it('builds the exact canvas skeleton and rebinds the runtime to replacement hosts', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI);

    const first = document.createElement('div');
    document.body.append(first);
    panel?.build?.(first, { spec: {} });
    expect([...first.children].filter((element) => element.tagName !== 'STYLE').map((element) => element.id))
      .toEqual(['tl-head', 'tl-canvas-wrap']);
    expect(first.querySelector('#tl-canvas-wrap > #tl-canvas')).not.toBeNull();
    expect(PM.TL.cv).toBe(first.querySelector('#tl-canvas'));

    const second = document.createElement('div');
    document.body.append(second);
    panel?.build?.(second, { spec: {} });
    expect(PM.TL.cv).toBe(second.querySelector('#tl-canvas'));
    expect(second.querySelector('.tl-transport, button, .iconbtn')).not.toBeNull();
  });

  it('renders the move slot synchronously before the layout injects its handle', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);

    panel?.build?.(body, { spec: { id: 'timeline' } });
    const slot = body.querySelector('#tl-head')!;
    const handle = document.createElement('button');
    handle.className = 'panel-move-handle';
    slot.insertBefore(handle, slot.firstChild);

    expect(slot.firstElementChild).toBe(handle);
    expect(slot.querySelector('.tl-transport')).not.toBeNull();
  });

  it('registers and runs a disposer for kernel reload or disable', () => {
    let panel: PanelDefinition | undefined;
    let dispose: (() => void) | undefined;
    const PM = timelinePM();
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) },
      onDispose: vi.fn((handler: () => void) => void (dispose = handler)),
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });
    const snap = body.querySelector<HTMLButtonElement>('button[title="Snapping (S)"]')!;
    const before = PM.snap;

    expect(dispose).toEqual(expect.any(Function));
    dispose?.();
    snap.click();

    expect(PM.TL.__timelineRuntimeDisposed).toBe(true);
    expect(PM.snap).toBe(before);
    expect(PM.__timelineBusReleases.length).toBeGreaterThan(0);
    expect(PM.__timelineBusReleases.every((release: ReturnType<typeof vi.fn>) => release.mock.calls.length === 1)).toBe(true);
  });

  it('reactivates in place with fresh handlers and preserved view state', () => {
    const PM = timelinePM();
    let firstPanel: PanelDefinition | undefined;
    let firstDispose: (() => void) | undefined;
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (firstPanel = definition)) },
      onDispose: vi.fn((handler: () => void) => void (firstDispose = handler)),
    } as unknown as PowermoveAPI);
    const firstBody = document.createElement('div');
    document.body.append(firstBody);
    firstPanel?.build?.(firstBody, { spec: {} });
    const timeline = PM.TL;
    timeline.pps = 246;

    firstDispose?.();
    let replacementPanel: PanelDefinition | undefined;
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (replacementPanel = definition)) },
      onDispose: vi.fn(),
    } as unknown as PowermoveAPI);
    const replacementBody = document.createElement('div');
    document.body.append(replacementBody);
    replacementPanel?.build?.(replacementBody, { spec: {} });

    expect(PM.TL).toBe(timeline);
    expect(PM.TL.pps).toBe(246);
    expect(PM.TL.__timelineRuntimeDisposed).toBe(false);
    expect(PM.TL.cv).toBe(replacementBody.querySelector('#tl-canvas'));
    expect(replacementBody.querySelectorAll('.tl-transport')).toHaveLength(1);
  });
});
