// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI } from 'powermove';

import activate, { splitSelectedLayersAtPlayhead, toggleLayerStrips } from './index';

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
    allProps: (layer: any) => Object.entries(layer.p ?? {}).map(([key, prop]) => ({ key, prop })),
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
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({} as never));
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

  it('binds M to open mixed layer strips together and collapse an open selection', () => {
    const layers = [
      { id: 'layer-1', collapsed: true },
      { id: 'layer-2', collapsed: false },
    ];
    const PM = {
      selLayers: () => layers,
      UIState: {
        getLayerCollapsed: (layer: any) => layer.collapsed,
        setLayerCollapsed: (layer: any, collapsed: boolean) => { layer.collapsed = collapsed; }
      },
      invalidate: vi.fn()
    };

    toggleLayerStrips(PM);
    expect(layers.map((layer) => layer.collapsed)).toEqual([false, false]);
    toggleLayerStrips(PM);
    expect(layers.map((layer) => layer.collapsed)).toEqual([true, true]);
    expect(PM.invalidate).toHaveBeenCalledWith('timeline');
  });

  it('selects only the new right-hand segments after splitting', () => {
    const left = { id: 'left', from: 2, dur: 6, d: {}, p: { x: { kf: [{ t: 0, v: 10 }, { t: 6, v: 70 }] } } };
    const project = { layers: [left] };
    let selected: string[] = [left.id];
    const PM = {
      allProps: (layer: any) => [{ prop: layer.p.x }],
      time: 5,
      proj: project,
      selLayers: () => [left],
      cloneLayer: (layer: any) => ({ ...structuredClone(layer), id: 'right' }),
      MediaTiming: { isTimed: () => false },
      hist: { do: (_label: string, run: () => void) => run() },
      bus: { emit: vi.fn() },
      ProjectIndex: { invalidate: vi.fn() },
      selectLayers: (ids: string[]) => { selected = ids; }
    };

    expect(splitSelectedLayersAtPlayhead(PM)).toEqual(['right']);
    expect(project.layers.map((layer) => ({ id: layer.id, from: layer.from, dur: layer.dur }))).toEqual([
      { id: 'right', from: 5, dur: 3 },
      { id: 'left', from: 2, dur: 3 }
    ]);
    expect(project.layers[0].p.x.kf).toEqual([{ t: -3, v: 10 }, { t: 3, v: 70 }]);
    expect(left.p.x.kf).toEqual([{ t: 0, v: 10 }, { t: 6, v: 70 }]);
    expect(PM.ProjectIndex.invalidate).toHaveBeenCalledOnce();
    expect(selected).toEqual(['right']);
  });

  it('registers the M timeline keybinding outside text fields', () => {
    let command: Record<string, any> | undefined;
    let binding: Record<string, any> | undefined;
    activate({
      host: { pm: { h: vi.fn(), clamp: vi.fn(), bus: { on: vi.fn() }, invalidate: vi.fn() } },
      panels: { register: vi.fn() },
      commands: { register: vi.fn((definition) => void (command = definition)) },
      keybindings: { bind: vi.fn((definition) => void (binding = definition)) }
    } as unknown as PowermoveAPI);

    expect(command).toMatchObject({ id: 'toggleLayerStrips', kb: 'M' });
    expect(binding).toEqual({ key: 'm', command: 'toggleLayerStrips', priority: 90 });
    expect(binding).not.toHaveProperty('inFields');
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

  it('refreshes timecode and graph state when switching projects or changing frame rate', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    PM.tc = (time: number, fps: number) => `${time}:${fps}`;
    activate({
      host: { pm: PM },
      panels: { register: (definition: PanelDefinition) => { panel = definition; } }
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });
    const emit = (name: string) => PM.bus.on.mock.calls
      .filter(([event]: [string]) => event === name)
      .forEach(([, callback]: [string, () => void]) => callback());
    PM.time = 2;
    emit('time');
    expect(body.querySelector('#tl-time')?.textContent).toBe('2:30');
    PM.time = 0;
    PM.proj.fps = 24;
    PM.TL.graph = true;
    emit('project');
    expect(body.querySelector('#tl-time')?.textContent).toBe('0:24');
    expect(body.querySelector('.tl-graph-slot button')?.classList.contains('on')).toBe(true);
  });

  it('scrubs time from the initial position instead of accumulating pointer offsets', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    let move: (dx: number) => void = () => {};
    PM.drag = (_event: unknown, options: any) => { move = options.move; };
    PM.setTime = (time: number) => { PM.time = time; };
    activate({
      host: { pm: PM },
      panels: { register: (definition: PanelDefinition) => { panel = definition; } }
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });
    PM.time = 2;
    body.querySelector('#tl-time')!.dispatchEvent(new PointerEvent('pointerdown', { button: 0 }));
    move(120);
    move(120);
    expect(PM.time).toBeCloseTo(2 + 10 / 30);
    move(0);
    expect(PM.time).toBe(2);
  });

  it('places only the essential controls in the ruler gutter', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });

    const graph = body.querySelector<HTMLButtonElement>('button[title="Graph editor (Shift+F3)"]')!;
    const slot = graph.closest('.tl-graph-slot')!;
    expect(slot.firstElementChild).toBe(graph);
    expect(slot.previousElementSibling?.classList.contains('tl-transport')).toBe(true);
    expect(graph.closest('.tl-transport')).toBeNull();
    expect(graph.querySelector('[data-icon="bezier"]')).not.toBeNull();
    expect(body.querySelector('.tl-view button[title="Graph editor (Shift+F3)"]')).toBeNull();
    expect(body.querySelector('button[title="Snapping (S)"]')).toBeNull();
    expect(body.querySelector('button[title="Previous edge"]')).toBeNull();
    expect(body.querySelector('button[title="Next edge"]')).toBeNull();
    expect(body.querySelector('button[title="Frame entire composition (⇧F)"]')).toBeNull();
    expect(body.querySelector('button[title="Loop"]')).toBeNull();
    expect(body.querySelector('button[title="Play / Pause (Space)"]')).not.toBeNull();
    expect(body.querySelector('input[aria-label="Timeline zoom"]')).toBeNull();
    expect(body.querySelector('#tl-time')).not.toBeNull();
    expect(body.querySelector<HTMLDivElement>('#tl-head')!.style.getPropertyValue('--tl-gutter')).toBe('224px');
    expect(body.querySelector<HTMLDivElement>('#tl-head')!.style.getPropertyValue('--tl-ruler')).toBe('28px');
  });

  it('preserves the live panel move handle when the extension rebuilds in place', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    activate({
      host: { pm: PM },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });
    const handle = document.createElement('button');
    handle.className = 'panel-move-handle inline';
    body.querySelector('#tl-head')!.prepend(handle);

    panel?.build?.(body, { spec: {} });

    expect(body.querySelector('#tl-head')?.firstElementChild).toBe(handle);
    expect(body.querySelectorAll('.panel-move-handle')).toHaveLength(1);
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
    const graph = body.querySelector<HTMLButtonElement>('button[title="Graph editor (Shift+F3)"]')!;
    const before = PM.TL.graph;

    expect(dispose).toEqual(expect.any(Function));
    dispose?.();
    graph.click();

    expect(PM.TL.__timelineRuntimeDisposed).toBe(true);
    expect(PM.TL.graph).toBe(before);
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

  it.each(['layer', 'prop', 'graph', 'below'])('empty %s clicks clear selection without seeking; modified clicks preserve selection', (area) => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    const layer = { id: 'layer-1', from: 2, dur: 3, p: {} };
    const prop = { kf: [] };
    PM.proj.layers = [layer];
    PM.sel = { layers: [layer.id], keys: ['key-1'], chan: '' };
    PM.bus.emit = vi.fn();
    PM.closeMenus = vi.fn();
    PM.selectLayers = vi.fn((layers) => { PM.sel.layers = layers; });
    PM.drag = vi.fn((_event, handlers) => handlers.up());
    activate({
      host: { pm: PM },
      panels: { register: (definition: PanelDefinition) => { panel = definition; } },
      onDispose: vi.fn(),
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });
    const T = PM.TL;
    T.rows = area === 'below' ? [] : [{ kind: area === 'prop' ? 'prop' : 'layer', L: layer, key: 'opacity', prop }];
    T.graph = area === 'graph';
    T._graph = { target: { L: layer, prop }, series: [{ prop }] };
    for (const modifier of ['none', 'shiftKey', 'metaKey']) {
      PM.sel.layers = [layer.id];
      PM.sel.keys = ['key-1'];
      const x = T.gut + T.pps; // 1s, before the clip or any key.
      // The graph reserves 28px below the ruler for its toolbar.
      const y = T.ruler + (area === 'graph' ? 28 : 0) + T.row / 2;
      const event = new PointerEvent('pointerdown', { clientX: x, clientY: y, [modifier]: true });
      Object.defineProperties(event, { offsetX: { value: x }, offsetY: { value: y } });
      T.cv.dispatchEvent(event);
      expect(PM.setTime).not.toHaveBeenCalled();
      expect(PM.sel.layers).toEqual(modifier === 'none' ? [] : [layer.id]);
      expect(PM.sel.keys).toEqual(modifier === 'none' ? [] : ['key-1']);
    }
  });

  it.each(['in', 'out'])('Shift-resizing the %s edge snaps and releasing Shift restores frame-only resizing', (side) => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    const layer = { id: 'clip', type: 'rect', from: 2, dur: 3, p: {}, d: {} };
    PM.proj.layers = [layer];
    PM.time = side === 'in' ? 3 : 6;
    PM.sel = { layers: [layer.id], keys: [], chan: '' };
    PM.bus.emit = vi.fn();
    PM.closeMenus = vi.fn();
    PM.selectLayers = vi.fn();
    PM.snapF = (n: number, fps: number) => Math.round(n * fps) / fps;
    PM.MediaTiming = { isTimed: () => false };
    PM.Edit = {
      begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(),
      dispatch: vi.fn(({ patch }) => {
        if (patch.from !== undefined) layer.from = patch.from;
        if (patch.duration !== undefined) layer.dur = patch.duration;
      }),
    };
    let drag: any;
    PM.drag = (_event: unknown, handlers: any) => { drag = handlers; return {}; };
    activate({
      host: { pm: PM },
      panels: { register: (definition: PanelDefinition) => { panel = definition; } },
      onDispose: vi.fn(),
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    document.body.append(body);
    panel?.build?.(body, { spec: {} });
    const T = PM.TL;
    T.pps = 100;
    T.scrollT = 0;
    T.rows = [{ kind: 'layer', L: layer }];
    const x = T.gut + (side === 'in' ? 2 : 5) * T.pps;
    const y = T.ruler + T.row / 2;
    const event = new PointerEvent('pointerdown', { clientX: x, clientY: y });
    Object.defineProperties(event, { offsetX: { value: x }, offsetY: { value: y } });
    T.cv.dispatchEvent(event);
    const edge = () => side === 'in' ? layer.from : layer.from + layer.dur;
    drag.move(95, 0, { shiftKey: true });
    expect(edge()).toBe(PM.time);
    drag.move(95, 0, { shiftKey: true });
    expect(edge()).toBe(PM.time);
    drag.move(95, 0, { shiftKey: false });
    expect(edge()).toBeCloseTo(PM.time - 1 / 30);
    drag.up();
    expect(PM.Edit.commit).toHaveBeenCalledExactlyOnceWith('Trim clip');
  });

  it('releases an external text field when the timeline starts a pointer gesture', () => {
    let panel: PanelDefinition | undefined;
    const PM = timelinePM();
    PM.bus.emit = vi.fn();
    PM.sel = { layers: [], keys: [], chan: '' };
    PM.closeMenus = vi.fn();
    PM.selectLayers = vi.fn();
    PM.drag = vi.fn((event: PointerEvent, handlers: { up?: () => void }) => {
      event.preventDefault();
      handlers.up?.();
      return { cancel: vi.fn() };
    });
    activate({
      host: { pm: PM },
      panels: { register: (definition: PanelDefinition) => { panel = definition; } },
      onDispose: vi.fn(),
    } as unknown as PowermoveAPI);
    const body = document.createElement('div');
    const composer = document.createElement('textarea');
    document.body.append(body, composer);
    panel?.build?.(body, { spec: {} });
    const canvas = body.querySelector<HTMLCanvasElement>('#tl-canvas')!;
    const timeline = PM.TL;
    timeline.rows = [];
    composer.focus();

    const x = timeline.gut + timeline.pps;
    const y = timeline.ruler + timeline.row / 2;
    const event = new PointerEvent('pointerdown', { button: 0, clientX: x, clientY: y, cancelable: true });
    Object.defineProperties(event, { offsetX: { value: x }, offsetY: { value: y } });
    canvas.dispatchEvent(event);

    expect(document.activeElement).not.toBe(composer);
    expect(PM.drag).toHaveBeenCalledOnce();
  });
});
