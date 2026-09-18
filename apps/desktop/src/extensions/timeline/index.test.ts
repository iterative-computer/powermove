// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, TimelineService } from 'powermove';

import activateExtension, { splitSelectedLayersAtPlayhead, toggleLayerStrips } from './index';
import { fakePowermoveAPI, type FakeAPIHarness } from './fake-api.test-helper';

const activeHarnesses: FakeAPIHarness[] = [];

function harness(): FakeAPIHarness {
  const value = fakePowermoveAPI(vi);
  activeHarnesses.push(value);
  return value;
}

function activate(value = harness()): FakeAPIHarness {
  activateExtension(value.api);
  return value;
}

function build(value: FakeAPIHarness): HTMLElement {
  const body = document.createElement('div');
  document.body.append(body);
  value.state.panel?.build?.(body, { spec: {} });
  return body;
}

function pointer(target: Element, x: number, y: number, init: PointerEventInit = {}): void {
  const event = new PointerEvent('pointerdown', { button: 0, clientX: x, clientY: y, ...init });
  Object.defineProperties(event, { offsetX: { value: x }, offsetY: { value: y } });
  target.dispatchEvent(event);
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
    for (const value of activeHarnesses.splice(0).reverse()) value.dispose();
    document.body.replaceChildren();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('registers the live timeline service and panel with automatic disposal', () => {
    const value = activate();
    expect(value.state.panel).toMatchObject({
      id: 'timeline', title: 'Timeline', flush: true, noscroll: true,
      headless: true, size: 340, moveSlot: '#tl-head',
    });
    expect(value.state.panel?.build).toEqual(expect.any(Function));
    expect(value.api.services.get<TimelineService>('timeline')).toBe(value.state.timeline);
    value.dispose();
    expect(value.api.services.get('timeline')).toBeNull();
  });

  it('binds M to open mixed layer strips together and collapse an open selection', () => {
    const value = harness();
    const layers = [
      { id: 'layer-1', collapsed: true },
      { id: 'layer-2', collapsed: false },
    ] as any[];
    value.state.project.layers = layers;
    value.state.selection.layers = layers.map((layer) => layer.id);

    toggleLayerStrips(value.api);
    expect(layers.map((layer) => layer.collapsed)).toEqual([false, false]);
    toggleLayerStrips(value.api);
    expect(layers.map((layer) => layer.collapsed)).toEqual([true, true]);
    expect(value.api.transport.invalidate).toHaveBeenCalledWith('timeline');
  });

  it('closes selected group hierarchies when M collapses their layer strips', () => {
    const value = harness();
    const group = { id: 'group-1', type: 'group', collapsed: false, groupCollapsed: false } as any;
    value.state.project.layers = [group];
    value.state.selection.layers = [group.id];

    toggleLayerStrips(value.api);

    expect(group.collapsed).toBe(true);
    expect(group.groupCollapsed).toBe(true);
    expect(value.api.uiState.setGroupCollapsed).toHaveBeenCalledWith(group, true);
  });

  it('selects only the new right-hand segments after splitting', () => {
    const value = harness();
    const left = { id: 'left', type: 'solid', name: 'Left', from: 2, dur: 6, d: {}, p: { x: { v: 10, kf: [{ i: 'a', t: 0, v: 10 }, { i: 'b', t: 6, v: 70 }] } } } as any;
    value.state.project.layers = [left];
    value.state.selection.layers = [left.id];
    vi.mocked(value.api.util.uid).mockReturnValueOnce('right').mockReturnValueOnce('right-a').mockReturnValueOnce('right-b');

    expect(splitSelectedLayersAtPlayhead(value.api)).toEqual([]);
    vi.mocked(value.api.anim.touch).mockClear();
    value.state.time = 5;
    expect(splitSelectedLayersAtPlayhead(value.api)).toEqual(['right']);
    expect(value.state.project.layers.map((layer: any) => ({ id: layer.id, from: layer.from, dur: layer.dur }))).toEqual([
      { id: 'right', from: 5, dur: 3 },
      { id: 'left', from: 2, dur: 3 },
    ]);
    expect(value.state.project.layers[0].p.x.kf.map((key: any) => key.t)).toEqual([-3, 3]);
    expect(left.p.x.kf.map((key: any) => key.t)).toEqual([0, 6]);
    expect(value.api.model.cloneLayer).toHaveBeenCalledWith(left);
    expect(value.api.anim.touch).toHaveBeenCalledOnce();
    expect(value.state.selection.layers).toEqual(['right']);
  });

  it('preserves source time when splitting timed media', () => {
    const value = harness();
    const left = { id: 'video', type: 'video', name: 'Video', from: 2, dur: 6, d: { trim: 1 }, p: {} } as any;
    value.state.project.layers = [left];
    value.state.selection.layers = [left.id];
    value.state.time = 5;
    vi.mocked(value.api.media.timing.isTimed).mockReturnValue(true);
    vi.mocked(value.api.media.timing.rate).mockReturnValue(2);

    splitSelectedLayersAtPlayhead(value.api);

    expect(value.state.project.layers[0].d.trim).toBe(7);
  });

  it('registers the M timeline keybinding outside text fields', () => {
    const value = activate();
    expect(value.api.commands.register).toHaveBeenCalledWith(expect.objectContaining({ id: 'toggleLayerStrips', kb: 'M' }));
    expect(value.api.keybindings.bind).toHaveBeenCalledWith({ key: 'm', command: 'toggleLayerStrips', priority: 90 });
    expect(vi.mocked(value.api.keybindings.bind).mock.calls[0]?.[0]).not.toHaveProperty('inFields');
  });

  it('owns the property-reveal and adjacent-keyframe command ids', () => {
    const value = harness();
    const layer = { id: 'layer-1', type: 'solid', from: 0, dur: 10, collapsed: true, reveal: null, d: {}, p: { 'position.x': { v: 0, kf: [{ i: 'key-1', t: 2, v: 10 }] } } } as any;
    value.state.project.layers = [layer];
    value.state.selection.layers = [layer.id];
    const commands = new Map<string, any>();
    vi.mocked(value.api.commands.register).mockImplementation((definition: any) => {
      commands.set(definition.id, definition);
      return { dispose: vi.fn() };
    });
    activate(value);

    expect([...commands.keys()].filter((id) => id.startsWith('timeline.revealProperty:'))).toEqual([
      'timeline.revealProperty:p', 'timeline.revealProperty:s', 'timeline.revealProperty:r',
      'timeline.revealProperty:t', 'timeline.revealProperty:a', 'timeline.revealProperty:u',
      'timeline.revealProperty:m', 'timeline.revealProperty:f', 'timeline.revealProperty:e',
      'timeline.revealProperty:l',
    ]);
    expect(commands.has('timeline.revealAll')).toBe(true);
    expect(commands.has('timeline.adjacentKeyframe:prev')).toBe(true);
    expect(commands.has('timeline.adjacentKeyframe:next')).toBe(true);
    expect(commands.has('split')).toBe(false);

    commands.get('timeline.revealProperty:p')?.run();
    expect(layer.reveal).toEqual(['position.x']);
    commands.get('timeline.adjacentKeyframe:next')?.run();
    expect(value.api.transport.setTime).toHaveBeenCalledWith(2);
  });

  it('builds the exact canvas skeleton and rebinds the runtime to replacement hosts', () => {
    const value = activate();
    const first = build(value);
    expect([...first.children].filter((element) => element.tagName !== 'STYLE').map((element) => element.id)).toEqual(['tl-head', 'tl-canvas-wrap']);
    expect(first.querySelector('#tl-canvas-wrap > #tl-canvas')).not.toBeNull();
    expect(value.state.timeline?.cv).toBe(first.querySelector('#tl-canvas'));

    const second = build(value);
    expect(value.state.timeline?.cv).toBe(second.querySelector('#tl-canvas'));
    expect(second.querySelector('.tl-transport, button, .iconbtn')).not.toBeNull();
  });

  it('refreshes timecode and graph state when switching projects or changing frame rate', () => {
    const value = activate();
    vi.mocked(value.api.util.tc).mockImplementation((time, fps) => `${time}:${fps}`);
    const body = build(value);
    vi.mocked(value.api.transport.invalidate).mockClear();
    value.state.time = 2;
    value.emit('time', 2);
    expect(body.querySelector('#tl-time')?.textContent).toBe('2:30');
    expect(value.api.transport.invalidate).toHaveBeenCalledWith('timeline');
    value.state.time = 0;
    value.state.project.fps = 24;
    if (!value.state.timeline) throw new Error('timeline service not registered');
    value.state.timeline.graph = true;
    value.emit('project:changed', { kind: 'project' });
    expect(body.querySelector('#tl-time')?.textContent).toBe('0:24');
    expect(body.querySelector('button[title="Graph editor (Shift+F3)"]')?.classList.contains('on')).toBe(true);
  });

  it('scrubs time from the initial position instead of accumulating pointer offsets', () => {
    const value = activate();
    let move: (dx: number) => void = () => {};
    vi.mocked(value.api.ui.drag).mockImplementation((_event: any, options: any) => {
      move = options.move;
      return { cancel: vi.fn() };
    });
    const body = build(value);
    value.state.time = 2;
    body.querySelector('#tl-time')?.dispatchEvent(new PointerEvent('pointerdown', { button: 0 }));
    move(120);
    move(120);
    expect(value.state.time).toBeCloseTo(2 + 10 / 30);
    move(0);
    expect(value.state.time).toBe(2);
  });

  it('places only the essential controls in the ruler gutter', () => {
    const value = activate();
    const body = build(value);
    const graph = body.querySelector<HTMLButtonElement>('button[title="Graph editor (Shift+F3)"]')!;
    const slot = graph.closest('.tl-graph-slot')!;
    expect(slot.lastElementChild).toBe(graph);
    expect(slot.firstElementChild).toBe(body.querySelector('button[title="Graph options"]'));
    expect(slot.previousElementSibling?.classList.contains('tl-transport')).toBe(true);
    expect(graph.closest('.tl-transport')).toBeNull();
    expect(graph.querySelector('[data-icon="bezier"]')).not.toBeNull();
    expect(body.querySelector('button[title="Snapping (S)"]')).toBeNull();
    expect(body.querySelector('button[title="Play / Pause (Space)"]')).not.toBeNull();
    expect(body.querySelector('input[aria-label="Timeline zoom"]')).toBeNull();
    expect(body.querySelector('#tl-time')).not.toBeNull();
    expect(body.querySelector<HTMLElement>('#tl-head')?.style.getPropertyValue('--tl-gutter')).toBe('224px');
    expect(body.querySelector<HTMLElement>('#tl-head')?.style.getPropertyValue('--tl-ruler')).toBe('28px');
  });

  it('preserves the live panel move handle when the extension rebuilds in place', () => {
    const value = activate();
    const body = build(value);
    const handle = document.createElement('button');
    handle.className = 'panel-move-handle inline';
    body.querySelector('#tl-head')?.prepend(handle);
    value.state.panel?.build?.(body, { spec: {} });
    expect(body.querySelector('#tl-head')?.firstElementChild).toBe(handle);
    expect(body.querySelectorAll('.panel-move-handle')).toHaveLength(1);
  });

  it('renders the move slot synchronously before the layout injects its handle', () => {
    const value = activate();
    const body = build(value);
    const slot = body.querySelector('#tl-head')!;
    const handle = document.createElement('button');
    handle.className = 'panel-move-handle';
    slot.insertBefore(handle, slot.firstChild);
    expect(slot.firstElementChild).toBe(handle);
    expect(slot.querySelector('.tl-transport')).not.toBeNull();
  });

  it('registers and runs a disposer for kernel reload or disable', () => {
    const value = activate();
    const body = build(value);
    const graph = body.querySelector<HTMLButtonElement>('button[title="Graph editor (Shift+F3)"]')!;
    const timeline = value.state.timeline as any;
    const before = timeline.graph;
    value.dispose();
    graph.click();
    expect(timeline.__timelineRuntimeDisposed).toBe(true);
    expect(timeline.graph).toBe(before);
    expect(value.api.services.get('timeline')).toBeNull();
  });

  it('reactivates in place with fresh handlers and preserved view state', () => {
    const value = activate();
    build(value);
    const timeline = value.state.timeline as any;
    timeline.pps = 246;
    value.dispose();
    activateExtension(value.api);
    const replacement = build(value);
    expect(value.state.timeline).toBe(timeline);
    expect(value.state.timeline?.pps).toBe(246);
    expect((value.state.timeline as any).__timelineRuntimeDisposed).toBe(false);
    expect(value.state.timeline?.cv).toBe(replacement.querySelector('#tl-canvas'));
    expect(replacement.querySelectorAll('.tl-transport')).toHaveLength(1);
  });

  it.each(['layer', 'prop', 'graph', 'below'])('empty %s clicks clear selection without seeking; modified clicks preserve selection', (area) => {
    const value = harness();
    const layer = { id: 'layer-1', type: 'solid', from: 2, dur: 3, d: {}, p: {} } as any;
    const prop = { v: 0, kf: [] };
    value.state.project.layers = [layer];
    value.state.selection = { layers: [layer.id], keys: ['key-1'], chan: null };
    vi.mocked(value.api.ui.drag).mockImplementation((_event: any, handlers: any) => {
      handlers.up?.();
      return { cancel: vi.fn() };
    });
    activate(value);
    const canvas = build(value).querySelector<HTMLCanvasElement>('#tl-canvas')!;
    const timeline = value.state.timeline as any;
    timeline.rows = area === 'below' ? [] : [{ kind: area === 'prop' ? 'prop' : 'layer', L: layer, key: 'opacity', prop }];
    timeline.graph = area === 'graph';
    timeline._graph = { target: { L: layer, prop }, series: [{ prop }], points: [] };
    for (const modifier of ['none', 'shiftKey', 'metaKey'] as const) {
      value.state.selection.layers = [layer.id];
      value.state.selection.keys = ['key-1'];
      vi.mocked(value.api.transport.setTime).mockClear();
      const x = timeline.gut + timeline.pps;
      const y = timeline.ruler + (area === 'graph' ? 28 : 0) + timeline.row / 2;
      pointer(canvas, x, y, modifier === 'none' ? {} : { [modifier]: true });
      expect(value.api.transport.setTime).not.toHaveBeenCalled();
      expect(value.state.selection.layers).toEqual(modifier === 'none' ? [] : [layer.id]);
      expect(value.state.selection.keys).toEqual(modifier === 'none' ? [] : ['key-1']);
    }
  });

  it.each(['in', 'out'] as const)('Shift-resizing the %s edge snaps and releasing Shift restores frame-only resizing', (side) => {
    const value = harness();
    const layer = { id: 'clip', type: 'solid', name: 'Clip', from: 2, dur: 3, d: {}, p: {} } as any;
    value.state.project.layers = [layer];
    value.state.selection.layers = [layer.id];
    value.state.time = side === 'in' ? 3 : 6;
    vi.mocked(value.api.edit.dispatch).mockImplementation((command: any) => {
      if (command.patch.from !== undefined) layer.from = command.patch.from;
      if (command.patch.duration !== undefined) layer.dur = command.patch.duration;
      return { ok: true } as any;
    });
    let drag: any;
    vi.mocked(value.api.ui.drag).mockImplementation((_event: any, handlers: any) => {
      drag = handlers;
      return { cancel: vi.fn() };
    });
    activate(value);
    const canvas = build(value).querySelector<HTMLCanvasElement>('#tl-canvas')!;
    const timeline = value.state.timeline as any;
    timeline.pps = 100;
    timeline.scrollT = 0;
    timeline.rows = [{ kind: 'layer', L: layer }];
    const x = timeline.gut + (side === 'in' ? 2 : 5) * timeline.pps;
    const y = timeline.ruler + timeline.row / 2;
    pointer(canvas, x, y);
    const edge = (): number => side === 'in' ? layer.from : layer.from + layer.dur;
    drag.move(95, 0, { shiftKey: true });
    expect(edge()).toBe(value.state.time);
    drag.move(95, 0, { shiftKey: true });
    expect(edge()).toBe(value.state.time);
    drag.move(95, 0, { shiftKey: false });
    expect(edge()).toBeCloseTo(value.state.time - 1 / 30);
    drag.up();
    expect(value.api.edit.commit).toHaveBeenCalledExactlyOnceWith('Trim clip');
  });

  it('releases an external text field when the timeline starts a pointer gesture', () => {
    const value = harness();
    vi.mocked(value.api.ui.drag).mockImplementation((event: PointerEvent, handlers: any) => {
      event.preventDefault();
      handlers.up?.();
      return { cancel: vi.fn() };
    });
    activate(value);
    const body = build(value);
    const composer = document.createElement('textarea');
    document.body.append(composer);
    const canvas = body.querySelector<HTMLCanvasElement>('#tl-canvas')!;
    const timeline = value.state.timeline as any;
    timeline.rows = [];
    composer.focus();
    pointer(canvas, timeline.gut + timeline.pps, timeline.ruler + timeline.row / 2, { cancelable: true });
    expect(document.activeElement).not.toBe(composer);
    expect(value.api.ui.drag).toHaveBeenCalledOnce();
  });

  it.each(['timecode', 'toolbar gutter'])('releases an external text field when the %s starts a pointer gesture', (area) => {
    const value = harness();
    vi.mocked(value.api.ui.drag).mockImplementation((event: PointerEvent) => {
      event.preventDefault();
      return { cancel: vi.fn() };
    });
    activate(value);
    const body = build(value);
    const composer = document.createElement('textarea');
    document.body.append(composer);
    composer.focus();

    const target = area === 'timecode'
      ? body.querySelector('#tl-time')!
      : body.querySelector('#tl-head')!;
    pointer(target, 0, 0, { bubbles: true, cancelable: true });

    expect(document.activeElement).not.toBe(composer);
  });
});
