import { describe, expect, it, vi } from 'vitest';
import { temporalEvaluate, temporalKeys } from './temporal-bridge';

describe('prepared temporal evaluation', () => {
  it('keeps legacy accessors when a track grows and rebinds them when keys move to another track', () => {
    const keys: any[] = [{ t: 0, v: 0 }, { t: 1, v: 100 }];
    temporalKeys(keys);
    keys[1].eo = [.5, .25];
    const getter = Object.getOwnPropertyDescriptor(keys[1], 'eo')!.get;
    keys.push({ t: 2, v: 300 });
    temporalKeys(keys);
    expect(Object.getOwnPropertyDescriptor(keys[1], 'eo')!.get).toBe(getter);
    expect(keys[1].eo).toEqual([.5, .125]);
    const moved = [keys[1], { t: 3, v: 200 }];
    temporalKeys(moved);
    expect(Object.getOwnPropertyDescriptor(keys[1], 'eo')!.get).not.toBe(getter);
    expect(keys[1].eo).toEqual([.5, .5]);
  });
  it('refreshes a same-time result after an edit and supports backward evaluation', () => {
    const keys = [{ t: 0, v: 0 }, { t: 1, v: 100 }];
    const spring = (t: number) => t;
    expect(temporalEvaluate(keys, .5, 0, spring)).toBe(50);
    expect(temporalEvaluate(keys, .5, 0, spring)).toBe(50);
    keys[1]!.v = 200;
    expect(temporalEvaluate(keys, .5, 1, spring)).toBe(100);
    expect(temporalEvaluate(keys, .75, 1, spring)).toBe(150);
    expect(temporalEvaluate(keys, .25, 1, spring)).toBe(50);
  });

  it('invalidates a cached value when a legacy handle setter edits the curve', () => {
    const keys: any[] = [{ t: 0, v: 0 }, { t: 1, v: 100 }];
    const spring = (t: number) => t;
    temporalKeys(keys);
    expect(temporalEvaluate(keys, .5, 0, spring)).toBe(50);
    keys[0].eo = [.8, 0];
    expect(temporalEvaluate(keys, .5, 0, spring)).toBeLessThan(50);
  });

  it('reuses spring results only for the same evaluator', () => {
    const keys = [{ t: 0, v: 0, eo: [.3, 0], spring: {} }, { t: 1, v: 100, ei: [.7, 1] }];
    const first = vi.fn(() => .2), second = vi.fn(() => .7);
    expect(temporalEvaluate(keys, .5, 0, first)).toBe(20);
    expect(temporalEvaluate(keys, .5, 0, first)).toBe(20);
    expect(first).toHaveBeenCalledTimes(1);
    expect(temporalEvaluate(keys, .5, 0, second)).toBe(70);
  });
});
