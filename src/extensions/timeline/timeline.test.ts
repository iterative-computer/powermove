import { afterEach, describe, expect, it, vi } from 'vitest';

import { createTimelineRuntime, shouldDrawClipLabel } from './timeline';
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
});
