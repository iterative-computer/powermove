import { describe, expect, it, vi } from 'vitest';
import { createGraphSampleCache } from './graph-sample-cache';

describe('graph curve samples', () => {
  it('reuses redraw samples and refreshes them after an edit, mode or project change', () => {
    const evaluate = vi.fn((axis, time, speed) => time + axis.prop.v + Number(speed));
    const cache = createGraphSampleCache(evaluate), project = {};
    const axis = { prop: { v: 10 }, L: { from: 0 }, key: 'opacity' };
    cache.begin(project, 'edit1:value');
    expect(cache.sample(axis, 1, false)).toBe(11);
    cache.begin(project, 'edit1:value');
    expect(cache.sample({ ...axis }, 1, false)).toBe(11);
    expect(evaluate).toHaveBeenCalledTimes(1);
    axis.prop.v = 20; cache.begin(project, 'edit2:value');
    expect(cache.sample(axis, 1, false)).toBe(21);
    cache.begin(project, 'edit2:speed');
    expect(cache.sample(axis, 1, true)).toBe(22);
    cache.begin({}, 'edit2:speed'); cache.sample(axis, 1, true);
    expect(evaluate).toHaveBeenCalledTimes(4);
  });

  it('keeps sampling correct when the cache budget is exhausted', () => {
    const evaluate = vi.fn((_axis, time) => time * 2), cache = createGraphSampleCache(evaluate, 2);
    const axis = { prop: {}, L: { from: 0 }, key: 'opacity' };
    cache.begin({}, '1');
    for (const time of [0, 1, 2, 0, 1, 2]) expect(cache.sample(axis, time, false)).toBe(time * 2);
    expect(evaluate).toHaveBeenCalledTimes(4);
  });
});
