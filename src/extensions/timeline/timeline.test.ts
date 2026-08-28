import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  applyKeyframeMovePlan,
  createTimelineRuntime,
  pickKeyframeHit,
  planKeyframeMove,
  shouldDrawClipLabel,
  type KeyframeMoveSnapshotItem,
} from './timeline';
import { expandScaleKeyIds, timelineProperties } from './property-tracks';
import { moveBezierHandle, visibleBezierHandle } from './bezier-drag';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';

function timelineRegistry(): Record<string, any> {
  vi.stubGlobal('window', {
    addEventListener() {},
    getComputedStyle() { return { getPropertyValue() { return ''; } }; }
  });
  vi.stubGlobal('document', { documentElement: { dataset: { theme: 'light' } } });

  const PM: Record<string, any> = {
    h() {},
    clamp(value: number, min: number, max: number) { return Math.max(min, Math.min(max, value)); },
    registerPanel() {},
    bus: { on() {} },
    invalidate() {}
  };
  createTimelineRuntime(PM);
  return PM;
}

afterEach(() => vi.unstubAllGlobals());

describe('timeline runtime', () => {
  it('groups Scale without altering unequal legacy key times or values', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim');
    PM.proj = PM.mkProject();
    const layer = PM.mkLayer('solid');
    PM.proj.layers = [layer];
    PM.setKey(layer, 'scale.x', 0, 100, 'power');
    PM.setKey(layer, 'scale.x', 2, 200, 'linear');
    PM.setKey(layer, 'scale.y', 0, 50, 'linear');
    PM.setKey(layer, 'scale.y', 1, 80, 'backOut');
    const before = JSON.stringify(layer.p);
    const rows = timelineProperties(PM, layer);
    const scale = rows.find(row => row.key === 'scale');
    expect(rows.filter(row => row.key.startsWith('scale'))).toHaveLength(1);
    expect(scale.label).toBe('Scale');
    expect(scale.prop.kf.map((key: any) => key.t)).toEqual([0, 1, 2]);
    expect(scale.prop.kf[0].members.map((member: any) => member.key.v)).toEqual([100, 50]);
    expect(expandScaleKeyIds(PM, [layer.p['scale.y'].kf[0].i])).toEqual([
      layer.p['scale.y'].kf[0].i, layer.p['scale.x'].kf[0].i
    ]);
    expect(expandScaleKeyIds(PM, [scale.prop.kf[1]])).toEqual([layer.p['scale.y'].kf[1].i]);
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

  it('keeps audio strips label-free without hiding other clip labels', () => {
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
    const math = timelineRegistry().TimelineWorkArea;
    const patch = math.resize([0, 10], 1, 14.5, 10, 1 / 30);

    expect(patch.duration).toBe(14.5);
    expect(patch.workArea).toEqual([0, 14.5]);
  });

  it('keeps work-area markers at least one frame apart', () => {
    const math = timelineRegistry().TimelineWorkArea;
    const left = math.resize([2, 8], 0, 20, 10, 1 / 30);
    const right = math.resize([2, 8], 1, 0, 10, 1 / 30);

    expect(left.workArea[0]).toBeCloseTo(8 - 1 / 30, 10);
    expect(right.workArea[1]).toBeCloseTo(2 + 1 / 30, 10);
  });

  it('preserves the span and stops a moved work area at composition edges', () => {
    const math = timelineRegistry().TimelineWorkArea;

    expect(math.move([2, 6], 3, 10, 1 / 30).workArea).toEqual([5, 9]);
    expect(math.move([2, 6], 20, 10, 1 / 30).workArea).toEqual([6, 10]);
    expect(math.move([2, 6], -20, 10, 1 / 30).workArea).toEqual([0, 4]);
  });

  it('restores the current project timeline session when activated after hydration', () => {
    const PM = timelineRegistry();
    delete PM.TL;
    PM.proj = { id: 'project-1' };
    PM.Projects = {
      getState: () => ({ timeline: { pps: 144, scrollT: 2.5, scrollY: 64, graph: true } })
    };

    const timeline = createTimelineRuntime(PM);

    expect(timeline).toMatchObject({ pps: 144, scrollT: 2.5, scrollY: 64, graph: true });
  });

  it('is idempotent within one module instance', () => {
    const PM = timelineRegistry();
    const first = PM.TL;
    const attachHead = first.attachHead;

    expect(createTimelineRuntime(PM)).toBe(first);
    expect(first.attachHead).toBe(attachHead);
  });

  it('disposes stale module closures while retaining the timeline state object', () => {
    const PM = timelineRegistry();
    const timeline = PM.TL;
    timeline.pps = 237;
    timeline.scrollT = 4.5;
    const staleAttach = timeline.attachHead;
    const originalDispose = timeline.disposeRuntime;
    const dispose = vi.fn(() => originalDispose());
    timeline.disposeRuntime = dispose;
    timeline.__timelineRuntimeToken = Symbol('stale-module');

    const replaced = createTimelineRuntime(PM);

    expect(dispose).toHaveBeenCalledOnce();
    expect(replaced).toBe(timeline);
    expect(replaced).toMatchObject({ pps: 237, scrollT: 4.5, __timelineRuntimeDisposed: false });
    expect(replaced.attachHead).not.toBe(staleAttach);
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
});
