// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';

import { createTimeline } from '../legacy/ui/timeline';
import { createViewer } from '../legacy/ui/viewer';
import TimelinePanel from './TimelinePanel.svelte';
import ViewerPanel from './ViewerPanel.svelte';
import { registerCanvasPanels } from './register-canvas';
import { unmountSveltePanels } from './registerSveltePanel';

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
    if (child == null) continue;
    element.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return element;
}

function canvasRegistry(): Record<string, any> {
  const PM: Record<string, any> = {
    h: domHelper,
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    bus: { on: vi.fn(() => () => {}) },
    GL: { gl: null, init: vi.fn() },
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
    invalidate: vi.fn(),
    PANELS: {},
  };
  PM.registerPanel = vi.fn((id: string, def: Record<string, any>) => {
    PM.PANELS[id] = { id, title: id, ...def };
  });
  return PM;
}

let instances: Array<Record<string, any>>;

beforeEach(() => {
  instances = [];
  document.body.replaceChildren();
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({} as CanvasRenderingContext2D));
  vi.stubGlobal('ResizeObserver', class {
    observe(): void {}
    disconnect(): void {}
    unobserve(): void {}
  });
});

afterEach(async () => {
  for (const instance of instances) await unmount(instance);
  unmountSveltePanels();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('canvas controllers', () => {
  it('attaches the viewer once to the provided legacy skeleton', () => {
    const PM = canvasRegistry();
    const stage = document.createElement('div');
    stage.id = 'stage';
    stage.innerHTML = '<div id="stage-inner"><canvas id="gl"></canvas><canvas id="overlay"></canvas></div>';
    const viewer = createViewer(PM);

    viewer.attach(stage);
    viewer.attach(stage);

    expect(PM.Viewer).toBe(viewer);
    expect(PM.GL.init).toHaveBeenCalledOnce();
    expect(PM.GL.init).toHaveBeenCalledWith(stage.querySelector('#gl'));
    expect(viewer.ov).toBe(stage.querySelector('#overlay'));
    // Hardened: a second host is ignored with a warning (HMR path) — the
    // original wiring must stay alive.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    viewer.attach(document.createElement('div'));
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('attach ignored'));
    warn.mockRestore();
  });

  it('attaches the timeline head and canvas once to the provided hosts', () => {
    const PM = canvasRegistry();
    const head = document.createElement('div');
    head.id = 'tl-head';
    const wrap = document.createElement('div');
    wrap.id = 'tl-canvas-wrap';
    wrap.innerHTML = '<canvas id="tl-canvas"></canvas>';
    const timeline = createTimeline(PM);

    timeline.attachHead(head);
    timeline.attachHead(head);
    timeline.attachCanvas(wrap);
    timeline.attachCanvas(wrap);

    expect(PM.TL).toBe(timeline);
    expect(head.querySelectorAll('.tl-group')).toHaveLength(2);
    expect(timeline.cv).toBe(wrap.querySelector('#tl-canvas'));
    // Hardened: the timeline rebinds to replacement hosts (HMR path).
    const head2 = document.createElement('div'); head2.id = 'tl-head';
    timeline.attachHead(head2);
    expect(head2.querySelector('.tl-transport, button, .iconbtn')).not.toBeNull();
    const wrap2 = document.createElement('div');
    const cv2 = document.createElement('canvas'); cv2.id = 'tl-canvas'; wrap2.appendChild(cv2);
    timeline.attachCanvas(wrap2);
  });
});

describe('canvas Svelte hosts', () => {
  it('renders the exact viewer and timeline DOM skeletons', () => {
    const PM = canvasRegistry();
    (window as any).PM = PM;
    const viewerTarget = document.createElement('div');
    const timelineTarget = document.createElement('div');
    document.body.append(viewerTarget, timelineTarget);

    instances.push(mount(ViewerPanel, { target: viewerTarget, props: { panelId: 'viewer', spec: {} } }));
    instances.push(mount(TimelinePanel, { target: timelineTarget, props: { panelId: 'timeline', spec: {} } }));
    flushSync();

    expect([...viewerTarget.children].map((element) => element.id)).toEqual(['stage']);
    expect(viewerTarget.querySelector('#stage > #stage-inner > #gl')).not.toBeNull();
    expect(viewerTarget.querySelector('#stage > #stage-inner > #overlay')).not.toBeNull();
    expect(viewerTarget.querySelectorAll('#stage, #stage-inner, #gl, #overlay')).toHaveLength(4);
    expect([...viewerTarget.querySelectorAll('#stage, #stage-inner, #gl, #overlay')].map((element) => element.className))
      .toEqual(['', '', '', '']);
    expect([...timelineTarget.children].map((element) => element.id)).toEqual(['tl-head', 'tl-canvas-wrap']);
    expect(timelineTarget.querySelector('#tl-canvas-wrap > #tl-canvas')).not.toBeNull();
    expect(timelineTarget.querySelectorAll('#tl-head, #tl-canvas-wrap, #tl-canvas')).toHaveLength(3);
    expect([...timelineTarget.querySelectorAll('#tl-head, #tl-canvas-wrap, #tl-canvas')].map((element) => element.className))
      .toEqual(['', '', '']);
  });
});

describe('registerCanvasPanels', () => {
  it('registers both persist panels with the exact legacy option sets', () => {
    const PM = canvasRegistry();

    registerCanvasPanels(PM);

    expect(PM.registerPanel).toHaveBeenCalledTimes(2);
    expect(PM.PANELS.viewer).toMatchObject({
      id: 'viewer', title: 'Composition', flush: true, noscroll: true, persist: true,
      headless: true, hideMoveHandle: true,
    });
    expect(PM.PANELS.viewer.size).toBeUndefined();
    expect(PM.PANELS.viewer.moveSlot).toBeUndefined();
    expect(PM.PANELS.timeline).toMatchObject({
      id: 'timeline', title: 'Timeline', flush: true, noscroll: true, persist: true,
      headless: true, size: 300, moveSlot: '#tl-head',
    });
    expect(PM.PANELS.timeline.hideMoveHandle).toBeUndefined();
  });

  it('renders the move slot synchronously before legacy layout injects its handle', () => {
    const PM = canvasRegistry();
    (window as any).PM = PM;
    registerCanvasPanels(PM);
    const body = document.createElement('div');

    PM.PANELS.timeline.build(body, { spec: { id: 'timeline' } });
    const slot = body.querySelector('#tl-head')!;
    const handle = document.createElement('button');
    handle.className = 'panel-move-handle';
    slot.insertBefore(handle, slot.firstChild);

    expect(slot.firstElementChild).toBe(handle);
    flushSync();
    expect(slot.querySelector('.tl-transport')).not.toBeNull();
  });
});
