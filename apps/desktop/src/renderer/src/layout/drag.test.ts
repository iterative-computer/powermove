// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../legacy/registry';
import { animatePanelLayout, beginPanelDrag, panelRects, retargetPreview } from './drag';
import type { DockSpec, PanelSpec } from './model';

const rect = (left: number, top: number, width = 100, height = 100): DOMRect =>
  ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON: () => ({}) }) as DOMRect;

beforeEach(() => {
  document.body.innerHTML = '';
  vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false })));
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('panel layout FLIP', () => {
  it('captures and animates only panels directly beneath body docks', () => {
    document.body.innerHTML = `
      <div id="body">
        <div class="dock" id="dock-center"><div class="panel" data-panel="live"></div></div>
        <div id="pm-panel-pool"><div class="panel" data-panel="parked"></div></div>
      </div>`;
    const live = document.querySelector<HTMLElement>('[data-panel="live"]')!;
    const parked = document.querySelector<HTMLElement>('[data-panel="parked"]')!;
    const liveRect = vi.fn(() => rect(10, 20));
    const parkedRect = vi.fn(() => rect(30, 40));
    live.getBoundingClientRect = liveRect;
    parked.getBoundingClientRect = parkedRect;

    const previous = panelRects();

    expect([...previous.keys()]).toEqual(['live']);
    expect(parkedRect).not.toHaveBeenCalled();

    liveRect.mockReturnValue(rect(40, 70));
    const liveAnimate = vi.fn();
    const parkedAnimate = vi.fn();
    Object.defineProperty(live, 'animate', { configurable: true, value: liveAnimate });
    Object.defineProperty(parked, 'animate', { configurable: true, value: parkedAnimate });

    animatePanelLayout(previous);

    expect(liveAnimate).toHaveBeenCalledWith(
      [
        { transform: 'translate3d(-30px,-50px,0)' },
        { transform: 'translate3d(0,0,0)' }
      ],
      { duration: 180, easing: 'cubic-bezier(.22,1,.36,1)' }
    );
    expect(parkedAnimate).not.toHaveBeenCalled();
  });
});

describe('drop preview retargeting', () => {
  it('restarts the on transition when the target key changes', () => {
    const callbacks: FrameRequestCallback[] = [];
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      callbacks.push(callback);
      return callbacks.length;
    });
    const preview = document.createElement('div');
    const target = { key: '' };
    preview.classList.add('on');
    document.body.appendChild(preview);

    retargetPreview(preview, target, 'virtual:left');
    expect(preview.classList.contains('on')).toBe(false);
    callbacks.shift()!(0);
    expect(preview.classList.contains('on')).toBe(true);

    retargetPreview(preview, target, 'virtual:left');
    expect(preview.classList.contains('on')).toBe(true);
    expect(callbacks).toHaveLength(0);

    retargetPreview(preview, target, 'dock-center:1');
    expect(preview.classList.contains('on')).toBe(false);
    callbacks.shift()!(16);
    expect(preview.classList.contains('on')).toBe(true);
  });

  it('retargets virtual and real previews and clears the key outside a drop zone', () => {
    document.body.innerHTML = `
      <div id="body">
        <div class="dock" id="dock-center"><div class="panel" data-panel="alpha"></div></div>
      </div>`;
    const body = document.getElementById('body')!;
    const dockElement = document.getElementById('dock-center')!;
    const panelElement = document.querySelector<HTMLElement>('[data-panel="alpha"]')!;
    body.getBoundingClientRect = () => rect(0, 0, 800, 600);
    dockElement.getBoundingClientRect = () => rect(100, 0, 600, 600);
    panelElement.getBoundingClientRect = () => rect(100, 0, 600, 200);

    let nextFrame = 0;
    const callbacks = new Map<number, FrameRequestCallback>();
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      callbacks.set(++nextFrame, callback);
      return nextFrame;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id) => callbacks.delete(id));
    const flushFrame = () => {
      const entry = callbacks.entries().next().value as [number, FrameRequestCallback] | undefined;
      expect(entry).toBeDefined();
      callbacks.delete(entry![0]);
      entry![1](entry![0] * 16);
    };

    let handlers!: {
      move: (dx: number, dy: number, event: PointerEvent) => void;
      up: (dx: number, dy: number, event: PointerEvent) => void;
      cancel: () => void;
    };
    const placements = [
      { dockId: 'left', index: 0 },
      { dockId: 'center', index: 0 },
      null,
      { dockId: 'center', index: 0 }
    ];
    const PM = {
      PANELS: { alpha: { title: 'Alpha' } },
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      Layout: {
        buildDockDropTargets: (docks: unknown[]) => [
          { id: 'left', virtual: true, rect: rect(0, 0, 96, 600), panels: [] },
          ...docks
        ],
        hitTestDockPlacement: vi.fn(() => placements.shift()),
        resolveDropIndex: vi.fn()
      },
      drag: vi.fn((_event, nextHandlers) => {
        handlers = nextHandlers;
        return { cancel: handlers.cancel };
      }),
      toast: vi.fn()
    } as unknown as PMRegistry;
    const pointer = (x: number) => ({ clientX: x, clientY: 100 }) as PointerEvent;
    const spec = { id: 'alpha' } as PanelSpec;
    const dock = { id: 'center', panels: [spec] } as DockSpec;

    beginPanelDrag(PM, pointer(10), spec, dock, panelElement);
    handlers.move(10, 0, pointer(10));
    flushFrame();
    const preview = document.querySelector<HTMLElement>('.panel-drop-preview')!;
    expect(preview.classList.contains('on')).toBe(false);
    expect(preview.classList.contains('dock-edge')).toBe(true);
    flushFrame();
    expect(preview.classList.contains('on')).toBe(true);

    handlers.move(20, 0, pointer(200));
    flushFrame();
    expect(preview.classList.contains('on')).toBe(false);
    expect(preview.classList.contains('dock-edge')).toBe(false);
    flushFrame();
    expect(preview.classList.contains('on')).toBe(true);

    handlers.move(30, 0, pointer(799));
    flushFrame();
    expect(preview.classList.contains('on')).toBe(false);

    handlers.move(40, 0, pointer(200));
    flushFrame();
    expect(preview.classList.contains('on')).toBe(false);
    flushFrame();
    expect(preview.classList.contains('on')).toBe(true);

    handlers.cancel();
  });
});
