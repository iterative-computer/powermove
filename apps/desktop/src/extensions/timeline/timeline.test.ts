// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  applyKeyframeMovePlan,
  createTimelineRuntime,
  pickKeyframeHit,
  planKeyframeMove,
  planQuickOffsetKeyframes,
  planQuickOffsetTiming,
  propertyValueColumns,
  resolveTimelineSnap,
  shouldDrawClipLabel,
  timelineWorkArea,
  toggleTimelineDisclosure,
  toggleTimelineScaleLink,
  type KeyframeMoveSnapshotItem,
} from './timeline';
import { expandScaleKeyIds, timelineProperties } from './property-tracks';
import {
  keysForBezierHandleDrag,
  materializeLinearBezierSegment,
  mirroredBezierHandlePoint,
  moveBezierHandle,
  visibleBezierHandle,
} from './bezier-drag';
import {
  graphSelectionBounds,
  graphTransformCursor,
  graphTransformHandleAtPoint,
  planGraphKeyframeMove,
  planGraphKeyframeScale,
  pointInGraphSelection,
  resolveGraphTarget,
  selectionAfterMarquee,
} from './graph-selection';
import { fakePowermoveAPI } from './fake-api.test-helper';

function timelineHarness() {
  const harness = fakePowermoveAPI(vi);
  harness.state.project = { id: 'project-1', w: 1920, h: 1080, fps: 30, dur: 10, work: [0, 10], layers: [], markers: [] };
  harness.state.timeline = createTimelineRuntime(harness.api);
  return harness;
}

afterEach(() => vi.unstubAllGlobals());

describe('timeline runtime', () => {
  it('opens a group hierarchy without opening its property strip', () => {
    const harness = timelineHarness();
    const group = { id: 'group', type: 'group', collapsed: true };
    const state = { groupCollapsed: true, layerCollapsed: true };
    vi.mocked(harness.api.uiState.getGroupCollapsed).mockImplementation(() => state.groupCollapsed);
    vi.mocked(harness.api.uiState.setGroupCollapsed).mockImplementation((_layer, value) => (state.groupCollapsed = value));
    vi.mocked(harness.api.uiState.getLayerCollapsed).mockImplementation(() => state.layerCollapsed);
    vi.mocked(harness.api.uiState.setLayerCollapsed).mockImplementation((_layer, value) => (state.layerCollapsed = value));

    expect(toggleTimelineDisclosure(harness.api, group)).toBe(false);
    expect(state).toEqual({ groupCollapsed: false, layerCollapsed: true });
    expect(group.collapsed).toBe(true);
  });

  it('never scrolls the timeline viewport to chase an external selection', () => {
    const harness = timelineHarness();
    const layers = Array.from({ length: 20 }, (_, index) => ({
      id: `layer-${index}`, type: 'text', name: `Layer ${index}`,
      group: null, shy: false, collapsed: true, p: {},
    }));
    harness.state.project.layers = layers;
    harness.state.selection.layers = [layers[0]!.id];
    (harness.state.timeline as any).hgt = 160;
    harness.state.timeline!.scrollY = 300;

    harness.emit('selection', harness.state.selection);

    expect(harness.state.timeline!.scrollY).toBe(300);
  });

  it('keeps existing property rows revealed when keyframing another property', () => {
    const harness = timelineHarness();
    const layer = { id: 'layer', type: 'solid', name: 'Layer', group: null, shy: false, collapsed: false, p: {} };
    const properties = [
      { key: 'position.y', label: 'Position Y', prop: { v: 540, kf: [{ i: 'position-key', t: 0, v: 540 }], expr: null } },
      { key: 'scale.x', label: 'Scale X', prop: { v: 95, kf: [{ i: 'scale-x-key', t: 0, v: 95 }], expr: null } },
      { key: 'scale.y', label: 'Scale Y', prop: { v: 95, kf: [{ i: 'scale-y-key', t: 0, v: 95 }], expr: null } },
    ];
    let reveal = properties.map(property => property.key);
    harness.state.project.layers = [layer];
    harness.state.selection.layers = [layer.id];
    vi.mocked(harness.api.anim.allProps).mockReturnValue(properties as any);
    vi.mocked(harness.api.uiState.getReveal).mockImplementation(() => reveal);
    vi.mocked(harness.api.uiState.setReveal).mockImplementation((_layer, keys) => (reveal = keys));

    harness.state.timeline!.reveal(layer as any, ['scale.x', 'scale.y']);

    expect(reveal).toEqual(['position.y', 'scale.x', 'scale.y']);
  });

  it('toggles linked Scale axes from the timeline with the standard layer command', () => {
    const harness = timelineHarness();
    const layer = { id: 'layer', scaleLinked: false };

    toggleTimelineScaleLink(harness.api, layer);

    expect(harness.api.edit.apply).toHaveBeenCalledExactlyOnceWith(
      { type: 'set_layer', target: 'layer', patch: { scaleLinked: true } },
      { label: 'Link scale axes', origin: 'timeline' },
    );
  });

  it('gives every property value a right-aligned column with a readable gap', () => {
    expect(propertyValueColumns(160, 280, 1)).toEqual([
      { left: 160, right: 272, width: 112 },
    ]);
    expect(propertyValueColumns(160, 280, 2, 24)).toEqual([
      { left: 160, right: 200, width: 40 },
      { left: 208, right: 248, width: 40 },
    ]);
  });

  it('groups Scale without altering unequal legacy key times or values', () => {
    const harness = fakePowermoveAPI(vi);
    const key = (i: string, t: number, v: number) => ({ i, t, v, eo: [0, 0], ei: [1, 1], hold: false });
    const layer: any = { id: 'solid', threeD: false, p: {
      'scale.x': { v: 100, expr: null, kf: [key('x0', 0, 100), key('x2', 2, 200)] },
      'scale.y': { v: 50, expr: null, kf: [key('y0', 0, 50), key('y1', 1, 80)] },
    } };
    harness.state.project.layers = [layer];
    const before = JSON.stringify(layer.p);
    const rows = timelineProperties(harness.api, layer);
    const scale = rows.find(row => row.key === 'scale');
    expect(rows.filter(row => row.key.startsWith('scale'))).toHaveLength(1);
    expect(scale.label).toBe('Scale');
    expect(scale.prop.kf.map((key: any) => key.t)).toEqual([0, 1, 2]);
    expect(scale.prop.kf[0].members.map((member: any) => member.key.v)).toEqual([100, 50]);
    expect(expandScaleKeyIds(harness.api, [layer.p['scale.y'].kf[0].i])).toEqual([
      layer.p['scale.y'].kf[0].i, layer.p['scale.x'].kf[0].i
    ]);
    expect(expandScaleKeyIds(harness.api, [scale.prop.kf[1]])).toEqual([layer.p['scale.y'].kf[1].i]);
    expect(JSON.stringify(layer.p)).toBe(before);
  });

  it.each([-120, 120])('moves incoming and outgoing Bézier handles with the pointer on a %s-pixel slope', (height) => {
    const begin: [number, number] = [100, 180];
    const end: [number, number] = [300, 180 + height];
    for (const start of [[.25, .2], [.75, .8]] as [number, number][]) {
      const next = moveBezierHandle(start, [20, -15], begin, end);
      expect((next[0] - start[0]) * 200).toBeCloseTo(20);
      expect((next[1] - start[1]) * height).toBeCloseTo(-15);
    }
  });

  it('keeps flat and zero-duration handle drags finite and clamps only time', () => {
    expect(moveBezierHandle([.5, .5], [20, 90], [0, 5], [100, 5])).toEqual([.7, .5]);
    expect(moveBezierHandle([.5, .5], [20, 90], [0, 5], [0, 5])).toEqual([.5, .5]);
    expect(moveBezierHandle([.5, .5], [200, -200], [0, 0], [100, 100])).toEqual([1, -1.5]);
  });

  it('exposes usable linear handles without modifying stored keys', () => {
    const a = { eo: [0, 0], ei: [1, 1] }, b = { eo: [0, 0], ei: [1, 1] };
    expect(visibleBezierHandle(a, b, 'eo')).toEqual([1 / 3, 1 / 3]);
    expect(visibleBezierHandle(b, a, 'ei')).toEqual([2 / 3, 2 / 3]);
    expect(a.eo).toEqual([0, 0]);
    expect(b.ei).toEqual([1, 1]);
  });

  it('materializes both linear handles before editing so neither point can disappear', () => {
    const previous = { eo: [0, 0] as [number, number], ei: [1, 1] as [number, number] };
    const next = { eo: [0, 0] as [number, number], ei: [1, 1] as [number, number] };
    expect(materializeLinearBezierSegment(previous, next)).toBe(true);
    expect(previous.eo).toEqual([1 / 3, 1 / 3]);
    expect(next.ei).toEqual([2 / 3, 2 / 3]);
    previous.eo = [.45, .1];
    expect(materializeLinearBezierSegment(previous, next)).toBe(false);
    expect(next.ei).toEqual([2 / 3, 2 / 3]);
  });

  it('rotates a continuous opposite handle while preserving its influence', () => {
    const mirrored = mirroredBezierHandlePoint([100, 100], [130, 140], [80, 100]);
    expect(Math.hypot(mirrored[0] - 100, mirrored[1] - 100)).toBeCloseTo(20);
    expect((mirrored[0] - 100) * 30 + (mirrored[1] - 100) * 40).toBeCloseTo(-1000);
  });

  it('builds an AE-style transform box and toggles Shift-marquee selection', () => {
    const bounds = graphSelectionBounds([{ id: 'a', x: 20, y: 30 }, { id: 'b', x: 80, y: 60 }]);
    expect(bounds).toEqual({ x0: 20, y0: 30, x1: 80, y1: 60 });
    expect(pointInGraphSelection(bounds, 50, 45)).toBe(true);
    expect(selectionAfterMarquee(['a', 'b'], ['b', 'c'], true)).toEqual(['a', 'c']);
    expect(selectionAfterMarquee(['a'], ['b', 'b'], false)).toEqual(['b']);
  });

  it('stands the transform box off its keys and names a grip per corner', () => {
    const bounds = graphSelectionBounds([{ id: 'a', x: 20, y: 30 }, { id: 'b', x: 80, y: 60 }], 16, 8)!;
    expect(bounds).toEqual({ x0: 12, y0: 22, x1: 88, y1: 68 });
    // The outermost keys sit inside the frame, so a grip never steals their press.
    expect(graphTransformHandleAtPoint(bounds, 20, 30)).toBeNull();
    expect(graphTransformHandleAtPoint(bounds, 12, 22)).toBe('nw');
    expect(graphTransformHandleAtPoint(bounds, 88, 45)).toBe('e');
    expect(graphTransformHandleAtPoint(bounds, 50, 68)).toBe('s');
    expect(graphTransformCursor('e')).toBe('ew-resize');
    expect(graphTransformCursor('ne')).toBe('nesw-resize');
  });

  it('scales a graph selection about an anchor and stops at the keys it does not own', () => {
    const items = [
      { id: 'a', property: 'p', time: 1, compositionTime: 1, value: 10, selected: true, maxTime: 10 },
      { id: 'b', property: 'p', time: 2, compositionTime: 2, value: 20, selected: true, maxTime: 10 },
      { id: 'fixed', property: 'p', time: 3, compositionTime: 3, value: 50, selected: false, maxTime: 10 },
    ];
    const doubled = planGraphKeyframeScale(items, {
      timeScale: 2, valueScale: 2, anchorTime: 1, anchorValue: 10,
    }, 10);
    // 'b' would land on 3s, where an unselected key already sits: one frame short.
    expect(doubled.timeScale).toBeCloseTo(1.9);
    expect(doubled.moves.map(move => move.time)).toEqual([1, 2.9]);
    expect(doubled.moves.map(move => move.value)).toEqual([10, 30]);

    // Collapsing keeps the pair a frame apart instead of stacking them.
    const collapsed = planGraphKeyframeScale(items, {
      timeScale: 0, valueScale: 1, anchorTime: 1, anchorValue: 10,
    }, 10);
    expect(collapsed.timeScale).toBeCloseTo(.1);
    expect(collapsed.moves.map(move => move.time)).toEqual([1, 1.1]);
  });

  it('anchors a scale in composition time for a layer that starts late', () => {
    const items = [
      { id: 'a', property: 'p', time: 0, compositionTime: 4, value: 0, selected: true, maxTime: 10 },
      { id: 'b', property: 'p', time: 2, compositionTime: 6, value: 8, selected: true, maxTime: 10 },
    ];
    const plan = planGraphKeyframeScale(items, {
      timeScale: .5, valueScale: -1, anchorTime: 4, anchorValue: 0,
    }, 30);
    expect(plan.moves.map(move => move.time)).toEqual([0, 1]);
    // A negative value scale flips the curve, which AE allows.
    expect(plan.moves.map(move => move.value)).toEqual([0, -8]);
  });

  it('keeps the focused curve when another layer strip is selected', () => {
    const focused = { kind: 'prop', key: 'opacity', L: { id: 'layer-a' }, prop: { kf: [{ i: 'a' }] } };
    const other = { kind: 'prop', key: 'opacity', L: { id: 'layer-b' }, prop: { kf: [{ i: 'b' }] } };

    expect(resolveGraphTarget(
      [focused, other],
      { layerId: 'layer-a', trackKey: 'opacity' },
      ['layer-b'],
      row => row.key === 'opacity',
    )).toBe(focused);
  });

  it('adjusts only the clicked Bézier handle even with multiple selected keys', () => {
    const first = { i: 'first', t: 0 };
    const second = { i: 'second', t: 1 };
    const third = { i: 'third', t: 2 };

    expect(keysForBezierHandleDrag([first, second, third], first, ['first', 'second'])).toEqual([first]);
    expect(keysForBezierHandleDrag([first, second, third], third, ['first', 'second'])).toEqual([third]);
    const paired = { i: 'paired', t: 0 };
    expect(keysForBezierHandleDrag([first, paired, second], first, ['first', 'second'])).toEqual([first, paired]);
  });

  it('labels every clip except audio, which shows its waveform alone', () => {
    expect(shouldDrawClipLabel('audio')).toBe(false);
    expect(shouldDrawClipLabel('video')).toBe(true);
    expect(shouldDrawClipLabel('image')).toBe(true);
    expect(shouldDrawClipLabel('text')).toBe(true);
  });

  it('prefers an already-selected keyframe when hit targets overlap', () => {
    const behind = { i: 'selected', x: 100 };
    const top = { i: 'top', x: 100 };
    const distance = (key: typeof behind) => Math.abs(key.x - 100);

    expect(pickKeyframeHit([top, behind], ['selected'], distance, 6)).toBe(behind);
    expect(pickKeyframeHit([top, behind], [], distance, 6)).toBe(top);
    expect(pickKeyframeHit([top, behind], ['elsewhere'], distance, 6)).toBe(top);
    const grouped = { i: 'scale-x', x: 100, members: [{ key: { i: 'scale-y' } }] };
    expect(pickKeyframeHit([top, grouped], ['scale-y'], distance, 6)).toBe(grouped);
  });

  it('only preserves selection when that key is under the pointer', () => {
    const selected = { i: 'selected', x: 100 };
    const other = { i: 'other', x: 120 };

    expect(pickKeyframeHit([selected, other], ['selected'], (key) => Math.abs(key.x - 120), 6)).toBe(other);
  });

  it('extends the composition when the out marker passes its end', () => {
    const patch = timelineWorkArea.resize([0, 10], 1, 14.5, 10, 1 / 30);

    expect(patch.duration).toBe(14.5);
    expect(patch.workArea).toEqual([0, 14.5]);
  });

  it('keeps work-area markers at least one frame apart', () => {
    const left = timelineWorkArea.resize([2, 8], 0, 20, 10, 1 / 30);
    const right = timelineWorkArea.resize([2, 8], 1, 0, 10, 1 / 30);

    expect(left.workArea[0]).toBeCloseTo(8 - 1 / 30, 10);
    expect(right.workArea[1]).toBeCloseTo(2 + 1 / 30, 10);
  });

  it('preserves the span and stops a moved work area at composition edges', () => {
    expect(timelineWorkArea.move([2, 6], 3, 10, 1 / 30).workArea).toEqual([5, 9]);
    expect(timelineWorkArea.move([2, 6], 20, 10, 1 / 30).workArea).toEqual([6, 10]);
    expect(timelineWorkArea.move([2, 6], -20, 10, 1 / 30).workArea).toEqual([0, 4]);
  });

  it('restores the current project timeline session when activated after hydration', () => {
    const harness = fakePowermoveAPI(vi);
    harness.state.project.id = 'project-1';
    harness.api.storage.set('session:project-1', { pps: 144, scrollT: 2.5, scrollY: 64, graph: true });
    const timeline = createTimelineRuntime(harness.api);

    expect(timeline).toMatchObject({ pps: 144, scrollT: 2.5, scrollY: 64, graph: true });
  });

  it('reuses the mounted runtime when the kernel activates with a fresh API facade', () => {
    /* A second runtime over the same canvas kept its own pps/scrollT and both
       painted #tl-canvas, so the track area flashed an older zoom/scroll for a
       frame before the live runtime repainted. */
    const wrap = document.createElement('div');
    wrap.id = 'tl-canvas-wrap';
    const canvas = document.createElement('canvas');
    canvas.id = 'tl-canvas';
    wrap.appendChild(canvas);
    document.body.appendChild(wrap);
    try {
      const mounted = timelineHarness();
      const runtime: any = mounted.state.timeline!;
      runtime.attachCanvas(wrap);
      runtime.pps = 144;
      runtime.scrollT = 5.25;

      const reactivated = fakePowermoveAPI(vi);
      reactivated.state.project.id = 'project-1';
      reactivated.api.storage.set('session:project-1', { pps: 90, scrollT: 3.2, scrollY: 0, graph: false });

      expect(createTimelineRuntime(reactivated.api)).toBe(runtime);
      expect(runtime).toMatchObject({ pps: 144, scrollT: 5.25 });
    } finally {
      wrap.remove();
    }
  });

  it('is idempotent within one module instance', () => {
    const harness = timelineHarness();
    const first: any = harness.state.timeline!;
    const attachHead = first.attachHead;

    expect(createTimelineRuntime(harness.api)).toBe(first);
    expect(first.attachHead).toBe(attachHead);
  });

  it('disposes stale module closures while retaining the timeline state object', () => {
    const harness = timelineHarness();
    const timeline: any = harness.state.timeline!;
    timeline.pps = 237;
    timeline.scrollT = 4.5;
    const staleAttach = timeline.attachHead;
    const originalDispose = timeline.disposeRuntime;
    const dispose = vi.fn(() => originalDispose());
    timeline.disposeRuntime = dispose;
    timeline.__timelineRuntimeToken = Symbol('stale-module');

    const replaced = createTimelineRuntime(harness.api);

    expect(dispose).toHaveBeenCalledOnce();
    expect(replaced).toBe(timeline);
    expect(replaced).toMatchObject({ pps: 237, scrollT: 4.5, __timelineRuntimeDisposed: false });
    expect(replaced.attachHead).not.toBe(staleAttach);
  });
});

describe('temporary Shift snapping', () => {
  it('acquires the nearest target inside the visual tolerance', () => {
    expect(resolveTimelineSnap(2.96, [1, 3, 3.08], 0.05)).toEqual({ time: 3, target: 3 });
  });

  it('holds an acquired target through nearby competition until release', () => {
    expect(resolveTimelineSnap(3.035, [3, 3.04], 0.05, 3, 0.08)).toEqual({ time: 3, target: 3 });
    expect(resolveTimelineSnap(3.2, [3, 3.04], 0.05, 3, 0.08)).toEqual({ time: 3.2, target: null });
  });

  it('leaves the playhead raw outside the acquire radius', () => {
    expect(resolveTimelineSnap(4.2, [3, 5], 0.05)).toEqual({ time: 4.2, target: null });
  });
});

describe('keyframe move planning', () => {
  const key = (
    id: string, property: string, time: number, selected = false, maxTime?: number,
  ): KeyframeMoveSnapshotItem<string> => ({ id, property, time, selected, ...(maxTime == null ? {} : { maxTime }) });

  it('uses one frame-snapped delta and collectively clamps at local time zero', () => {
    const items = [key('early', 'x', 0.5, true), key('late', 'y', 1.5, true)];
    const plan = planKeyframeMove(items, -2, 10);
    const moved = applyKeyframeMovePlan(items, plan);

    expect(plan.delta).toBe(-0.5);
    expect(moved.map((item) => item.time)).toEqual([0, 1]);
    expect(moved[1]!.time - moved[0]!.time).toBe(1);
  });

  it('snaps the shared delta instead of snapping and collapsing each key', () => {
    const items = [key('a', 'x', 0.03, true), key('b', 'x', 0.17, true)];
    const plan = planKeyframeMove(items, 0.06, 10);
    const moved = applyKeyframeMovePlan(items, plan);

    expect(plan.delta).toBe(0.1);
    expect(moved.map((item) => item.time)).toEqual([0.13, 0.27]);
    expect(moved[1]!.time - moved[0]!.time).toBeCloseTo(0.14, 10);
  });

  it('collectively clamps to the most constrained layer-local composition end', () => {
    const items = [
      key('roomy', 'x', 2, true, 8),
      key('constrained', 'y', 4, true, 5),
    ];
    const plan = planKeyframeMove(items, 3, 30);
    const moved = applyKeyframeMovePlan(items, plan);

    expect(plan.delta).toBe(1);
    expect(moved.map(({ id, time }) => ({ id, time }))).toEqual([
      { id: 'roomy', time: 3 },
      { id: 'constrained', time: 5 },
    ]);
    expect(moved[1]!.time - moved[0]!.time).toBe(2);
  });

  it('does not push a legacy out-of-range key farther right', () => {
    const legacy = key('legacy', 'x', 12, true, 12);
    const companion = key('companion', 'y', 2, true, 8);

    const right = applyKeyframeMovePlan(
      [legacy, companion],
      planKeyframeMove([legacy, companion], 2, 30),
    );
    const left = applyKeyframeMovePlan(
      [legacy, companion],
      planKeyframeMove([legacy, companion], -1, 30),
    );

    expect(right.map((item) => item.time)).toEqual([12, 2]);
    expect(left.map((item) => item.time)).toEqual([11, 1]);
  });

  it('lets the moved selected key replace an occupied destination by id', () => {
    const moving = key('moving', 'position.x', 1, true);
    const occupied = key('occupied', 'position.x', 2);
    const plan = planKeyframeMove([moving, occupied], 1, 30);
    const result = applyKeyframeMovePlan([moving, occupied], plan);

    expect(plan.removed.map((item) => item.id)).toEqual(['occupied']);
    expect(result).toEqual([{ ...moving, time: 2 }]);
    expect(result[0]!.id).toBe('moving');
    expect(result[0]!.selected).toBe(true);
  });

  it('restores the destination when a later pointer move leaves the collision', () => {
    const snapshot = [
      key('moving', 'position.x', 1, true),
      key('occupied', 'position.x', 2),
    ];
    const onCollision = applyKeyframeMovePlan(snapshot, planKeyframeMove(snapshot, 1, 30));
    const movedAway = applyKeyframeMovePlan(snapshot, planKeyframeMove(snapshot, 2, 30));

    expect(onCollision.map((item) => item.id)).toEqual(['moving']);
    expect(movedAway.map(({ id, time }) => ({ id, time }))).toEqual([
      { id: 'moving', time: 3 },
      { id: 'occupied', time: 2 },
    ]);
  });

  it('does not remove a key at the same frame in another property', () => {
    const moving = key('moving', 'position.x', 1, true);
    const sameProperty = key('same-property', 'position.x', 2);
    const otherProperty = key('other-property', 'position.y', 2);
    const plan = planKeyframeMove([moving, sameProperty, otherProperty], 1, 30);
    const result = applyKeyframeMovePlan([moving, sameProperty, otherProperty], plan);

    expect(plan.removed.map((item) => item.id)).toEqual(['same-property']);
    expect(result.map((item) => item.id)).toEqual(['moving', 'other-property']);
  });

  it('leaves legacy overlaps alone for a value-only graph edit', () => {
    const selected = key('selected', 'opacity', 1, true);
    const overlap = key('overlap', 'opacity', 1);
    const plan = planKeyframeMove([selected, overlap], 0, 30);

    expect(plan.removed).toEqual([]);
    expect(applyKeyframeMovePlan([selected, overlap], plan).map((item) => item.id))
      .toEqual(['selected', 'overlap']);
  });

  it('quick-offsets ordered layer groups from a fixed first item to the full dragged last item', () => {
    const plan = planQuickOffsetTiming([
      { time: 1, offsetIndex: 0, offsetCount: 4, minTime: 0 },
      { time: 1, offsetIndex: 1, offsetCount: 4, minTime: 0 },
      { time: 1, offsetIndex: 2, offsetCount: 4, minTime: 0 },
      { time: 1, offsetIndex: 3, offsetCount: 4, minTime: 0 },
    ], 3);

    expect(plan).toEqual({ total: 3, perGroup: 1, times: [1, 2, 3, 4] });
  });

  it('clamps a negative quick offset at zero without disturbing spacing inside a keyframe group', () => {
    const items = [
      { ...key('a', 'x', 1, true), offsetIndex: 0, offsetCount: 3 },
      { ...key('b', 'x', 2, true), offsetIndex: 1, offsetCount: 3 },
      { ...key('c', 'x', 4, true), offsetIndex: 1, offsetCount: 3 },
      { ...key('d', 'x', 1, true), offsetIndex: 2, offsetCount: 3 },
    ];
    const plan = planQuickOffsetKeyframes(items, -4, 30);

    expect(plan.delta).toBe(-1);
    expect(plan.moves.map(move => move.time)).toEqual([1, 1.5, 3.5, 0]);
    expect(plan.moves[2]!.time - plan.moves[1]!.time).toBe(2);
  });

  it('moves a graph selection together without deleting an occupied key', () => {
    const first = key('first', 'opacity', 1, true);
    const second = key('second', 'opacity', 2, true);
    const occupied = key('occupied', 'opacity', 4);
    const plan = planGraphKeyframeMove([first, second, occupied], 3, 10);

    expect(plan.delta).toBe(1.9);
    expect(plan.moves.map(move => move.time)).toEqual([2.9, 3.9]);
    expect(plan.removed).toEqual([]);
  });
});
