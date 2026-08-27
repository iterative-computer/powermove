import { afterEach, describe, expect, it, vi } from 'vitest';

import { createTimelineRuntime, shouldDrawClipLabel } from './timeline';

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
