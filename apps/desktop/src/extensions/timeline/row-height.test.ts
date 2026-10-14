// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { anchoredOffset, normalizeRowZoom, ROW_HEIGHT_LIMITS, rowHeight, rowZoomLimits, wheelPixels, wheelRowZoom } from './row-height';
import { createTimelineRuntime } from './timeline';
import { fakePowermoveAPI } from './fake-api.test-helper';
import { install as installUIState } from '../../renderer/src/legacy/core/ui-state';

describe('row height zoom', () => {
  it('keeps the workspace height at zoom 1 and the old track rule', () => {
    expect(rowHeight(30, 1, 'layers')).toBe(30);
    expect(rowHeight(26, 1, 'tracks')).toBe(34);
    expect(rowHeight(40, 1, 'tracks')).toBe(46);
  });

  it('clamps each view to its own bounds', () => {
    for (const mode of ['layers', 'tracks'] as const) {
      expect(rowHeight(30, .01, mode)).toBe(ROW_HEIGHT_LIMITS[mode].min);
      expect(rowHeight(30, 100, mode)).toBe(ROW_HEIGHT_LIMITS[mode].max);
      const [min, max] = rowZoomLimits(30, mode);
      expect(rowHeight(30, min, mode)).toBe(ROW_HEIGHT_LIMITS[mode].min);
      expect(rowHeight(30, max, mode)).toBe(ROW_HEIGHT_LIMITS[mode].max);
    }
  });

  it('normalizes mouse notches, line deltas, and horizontal Alt wheels', () => {
    expect(wheelPixels({ deltaX: 0, deltaY: 100, deltaMode: 0 }, 300)).toBe(100);
    expect(wheelPixels({ deltaX: 0, deltaY: 3, deltaMode: 1 }, 300)).toBe(99);
    expect(wheelPixels({ deltaX: 0, deltaY: 1, deltaMode: 2 }, 300)).toBe(300);
    expect(wheelPixels({ deltaX: -120, deltaY: 0, deltaMode: 0 }, 300)).toBe(-120);
    expect(wheelPixels({ deltaX: 2, deltaY: -5, deltaMode: 0 }, 300)).toBe(-5);
  });

  it('grows on wheel up, shrinks on wheel down, and reverses exactly', () => {
    const view = { base: 30, mode: 'layers' as const, page: 300 };
    const up = wheelRowZoom(1, { deltaX: 0, deltaY: -100, deltaMode: 0 }, view);
    expect(up).toBeGreaterThan(1.1);
    expect(up).toBeLessThan(1.25);
    expect(wheelRowZoom(up, { deltaX: 0, deltaY: 100, deltaMode: 0 }, view)).toBeCloseTo(1, 12);
    // A trackpad's small deltas move the zoom smoothly.
    const swipe = wheelRowZoom(1, { deltaX: 0, deltaY: -2, deltaMode: 0 }, view);
    expect(swipe).toBeGreaterThan(1);
    expect(swipe).toBeLessThan(1.01);
  });

  it('stops at the bounds without banking zoom past them', () => {
    const view = { base: 30, mode: 'layers' as const, page: 300 };
    let zoom = 1;
    for (let i = 0; i < 100; i++) zoom = wheelRowZoom(zoom, { deltaX: 0, deltaY: -100, deltaMode: 0 }, view);
    expect(zoom).toBe(rowZoomLimits(30, 'layers')[1]);
    const back = wheelRowZoom(zoom, { deltaX: 0, deltaY: 100, deltaMode: 0 }, view);
    expect(rowHeight(30, back, 'layers')).toBeLessThan(ROW_HEIGHT_LIMITS.layers.max);
  });

  it('ignores invalid stored zoom and anchors offsets', () => {
    expect(normalizeRowZoom(undefined)).toBe(1);
    expect(normalizeRowZoom(-2)).toBe(1);
    expect(normalizeRowZoom(Number.NaN)).toBe(1);
    expect(anchoredOffset(10, 40, 80)).toBe(20);
    // Below the header row, unfolded property lanes keep their height.
    expect(anchoredOffset(50, 40, 80)).toBe(90);
  });
});

const disposers: Array<() => void> = [];
afterEach(() => {
  disposers.splice(0).forEach(dispose => dispose());
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function harness(mode: 'layers' | 'tracks', options: { session?: unknown } = {}) {
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
  vi.spyOn(performance, 'now').mockReturnValue(0);
  const value = fakePowermoveAPI(vi);
  value.state.project.id = 'project-1';
  value.state.project.layers = [
    ...Array.from({ length: 40 }, (_, i) => ({
      id: `layer-${i}`, name: `Layer ${i}`, type: 'solid', collapsed: i !== 0,
      from: i % 2 ? 0 : 12 * Math.floor(i / 2), dur: 10, on: true, d: {}, fx: [], masks: [],
      // Overlapping clips stack onto separate tracks in the track view.
      p: { opacity: { v: 100, kf: [{ i: `key-${i}`, t: 0, v: 100 }, { i: `key-${i}b`, t: 1, v: 50 }] } },
    })),
    { id: 'cap', name: 'English', type: 'captions', collapsed: true, from: 0, dur: 10, on: true, d: { cues: [{ id: 'c1', start: 0, end: 2, text: 'Hello' }], style: {} }, fx: [], masks: [], p: {} },
  ];
  value.api.storage.set('mode', mode);
  if (options.session) value.api.storage.set('session:project-1', options.session);
  const PM: any = { proj: value.state.project, bus: { on() {} } };
  installUIState(PM);
  Object.assign(value.api.uiState, PM.UIState);
  const text = vi.fn();
  const context = new Proxy({ fillText: text, measureText: () => ({ width: 10 }), createLinearGradient: () => ({ addColorStop() {} }) }, {
    get(target, key) { return key in target ? target[key as keyof typeof target] : () => {}; },
    set(target, key, next) { (target as any)[key] = next; return true; },
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as any);
  const timeline: any = createTimelineRuntime(value.api);
  const wrap = document.createElement('div');
  wrap.id = 'tl-canvas-wrap';
  const canvas = document.createElement('canvas');
  canvas.id = 'tl-canvas';
  wrap.append(canvas); document.body.append(wrap);
  vi.spyOn(wrap, 'getBoundingClientRect').mockReturnValue({ width: 900, height: 400, left: 0, top: 0 } as DOMRect);
  timeline.attachCanvas(wrap);
  timeline.w = 900; timeline.hgt = 400;
  disposers.push(() => timeline.disposeRuntime());
  const draw = () => { text.mockClear(); value.emit('invalidate', 'timeline'); };
  draw();
  const wheel = (init: WheelEventInit, y: number, x = 400) => {
    const event = new WheelEvent('wheel', { cancelable: true, ...init });
    // happy-dom drops modifier and delta init fields on WheelEvent.
    Object.defineProperties(event, {
      offsetX: { value: x }, offsetY: { value: y },
      deltaX: { value: init.deltaX ?? 0 }, deltaY: { value: init.deltaY ?? 0 }, deltaMode: { value: init.deltaMode ?? 0 },
      altKey: { value: !!init.altKey }, ctrlKey: { value: !!init.ctrlKey }, metaKey: { value: !!init.metaKey }, shiftKey: { value: !!init.shiftKey },
    });
    canvas.dispatchEvent(event);
    return event;
  };
  return { ...value, timeline, text, draw, wheel };
}

/** Content row (fractional) under a canvas y in the layer view. */
const layerRowAt = (T: any, y: number) => (y - T.ruler + T.scrollY) / T.row;

describe('Alt + wheel over the timeline', () => {
  it('scales layer rows around the row under the pointer', () => {
    const value = harness('layers');
    const T = value.timeline;
    T.scrollY = 120; value.draw();
    const y = 230, before = layerRowAt(T, y), row = T.row;
    const event = value.wheel({ altKey: true, deltaY: -100 }, y);
    expect(event.defaultPrevented).toBe(true);
    expect(T.row).toBeGreaterThan(row);
    expect(layerRowAt(T, y)).toBeCloseTo(before, 6);
    value.wheel({ altKey: true, deltaY: 100 }, y);
    expect(T.row).toBe(row);
    expect(layerRowAt(T, y)).toBeCloseTo(before, 6);
    // Alt + wheel is not a scroll or a time pan.
    expect(T.scrollT).toBe(0);
  });

  it('accepts a horizontal Alt wheel and line-mode deltas', () => {
    const value = harness('layers');
    const T = value.timeline, row = T.row;
    value.wheel({ altKey: true, deltaX: -100 }, 200);
    expect(T.row).toBeGreaterThan(row);
    expect(T.scrollT).toBe(0);
    const grown = T.row;
    value.wheel({ altKey: true, deltaY: 3, deltaMode: 1 }, 200);
    expect(T.row).toBeLessThan(grown);
  });

  it('keeps the existing wheel bindings', () => {
    const value = harness('layers');
    const T = value.timeline, row = T.row, pps = T.pps;
    value.wheel({ ctrlKey: true, deltaY: -100 }, 200);
    expect(T.pps).toBeGreaterThan(pps);
    // Alt with Ctrl/Cmd (or a pinch) stays a horizontal zoom.
    value.wheel({ ctrlKey: true, altKey: true, deltaY: -100 }, 200);
    value.wheel({ shiftKey: true, deltaY: 50 }, 200);
    expect(T.scrollT).toBeGreaterThan(0);
    value.wheel({ deltaY: 40 }, 200);
    expect(T.scrollY).toBe(40);
    expect(T.row).toBe(row);
  });

  it('anchors the track under the pointer in the track view, including unfolded lanes', () => {
    const value = harness('tracks');
    const T = value.timeline;
    T.trackScrollY = 20;
    // Track chips are drawn at their lane's header centre (+.5).
    const chips = () => {
      value.draw();
      return value.text.mock.calls.filter(([label]) => /^[VAC]\d+$/.test(String(label)));
    };
    for (const y of [150, 260]) {
      const height = rowHeight(T.rowBase, T.rowZoom, 'tracks');
      const chip = chips().find(([, , cy]) => Math.abs(cy - .5 - y) < height / 2)!;
      expect(chip).toBeDefined();
      const offset = y - (chip[2] - .5 - height / 2);
      value.wheel({ altKey: true, deltaY: -200 }, y);
      const grown = rowHeight(T.rowBase, T.rowZoom, 'tracks');
      expect(grown).toBeGreaterThan(height);
      const after = chips().find(([label]) => label === chip[0])!;
      expect(y - (after[2] - .5 - grown / 2)).toBeCloseTo(offset * grown / height, 6);
    }
    // Over an unfolded clip's property lane, that lane stays put.
    const label = () => value.text.mock.calls.find(([text, x]) => text === 'opacity' && x < T.gut)?.[2];
    value.draw();
    const y = label();
    expect(y).toBeGreaterThan(T.ruler);
    value.wheel({ altKey: true, deltaY: -200 }, y);
    value.draw();
    expect(label()).toBeCloseTo(y, 6);
  });

  it('lays out rows with centred text at the minimum and maximum heights', () => {
    for (const mode of ['layers', 'tracks'] as const) {
      for (const deltaY of [5000, -5000]) {
        const value = harness(mode);
        const T = value.timeline;
        for (let i = 0; i < 20; i++) value.wheel({ altKey: true, deltaY }, 200);
        const height = rowHeight(T.rowBase, T.rowZoom, mode);
        expect(height).toBe(deltaY > 0 ? ROW_HEIGHT_LIMITS[mode].min : ROW_HEIGHT_LIMITS[mode].max);
        T.scrollY = 0;
        value.draw();
        if (mode === 'layers') {
          expect(T.row).toBe(height);
          const name = value.text.mock.calls.find(([label, x]) => label === 'Layer 0' && x < T.gut)!;
          expect(name[2]).toBeCloseTo(T.ruler + height / 2, 6);
          // Property sub-rows share the row height and stay centred.
          const prop = T.rows.findIndex((row: any) => row.kind === 'prop');
          expect(prop).toBeGreaterThan(0);
          const centre = T.ruler + prop * height + height / 2;
          expect(value.text.mock.calls.some(([, x, y]) => x < T.gut && y === centre)).toBe(true);
        } else {
          const chips = value.text.mock.calls.filter(([label]) => /^[VAC]\d+$/.test(String(label))).map(([, , cy]) => cy);
          const sorted = [...new Set(chips)].sort((a, b) => a - b);
          expect(sorted.length).toBeGreaterThan(2);
          // C1 sits on its own above the divider; consecutive video tracks are one height apart.
          expect(sorted[2]! - sorted[1]!).toBeGreaterThanOrEqual(height);
        }
        disposers.splice(0).forEach(dispose => dispose());
        vi.restoreAllMocks();
        document.body.replaceChildren();
      }
    }
  });

  it('persists the height with the project timeline session', () => {
    const value = harness('layers');
    value.wheel({ altKey: true, deltaY: -300 }, 200);
    const zoom = value.timeline.rowZoom;
    expect(zoom).toBeGreaterThan(1);
    disposers.splice(0).forEach(dispose => dispose());
    expect(value.api.storage.get<any>('session:project-1').rowZoom).toBe(zoom);
    vi.restoreAllMocks(); document.body.replaceChildren();
    const restored = harness('layers', { session: { pps: 90, scrollT: 0, scrollY: 0, rowZoom: zoom } });
    expect(restored.timeline.row).toBe(rowHeight(restored.timeline.rowBase, zoom, 'layers'));
  });
});
