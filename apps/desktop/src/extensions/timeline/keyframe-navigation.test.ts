import { describe, it, expect } from 'vitest';
import { adjacentKeyframe, keyframeTimes } from './keyframe-navigation';

describe('keyframe navigation', () => {
  const property = (key: string, times: number[]) => ({ key, prop: { kf: times.map(t => ({ t })) } });
  const pm = () => ({ time: 2, proj: { dur: 8, layers: [
    { id: 'a', from: 1, props: [property('position.x', [0, 2]), property('scale.x', [1]), property('scale.y', [3])] },
    { id: 'b', from: 0, props: [property('opacity', [1, 5, NaN, 20])] },
  ] }, sel: { layers: [] as string[], chan: null as string | null }, allProps: (L: any) => L.props });
  it('uses real keys, offsets, finite composition times and both Scale channels', () => {
    const PM = pm();
    expect(keyframeTimes(PM)).toEqual([1, 2, 3, 4, 5]);
    PM.sel.layers = ['a']; PM.sel.chan = 'scale';
    expect(keyframeTimes(PM)).toEqual([2, 4]);
    expect(adjacentKeyframe(PM, -1)).toBeUndefined();
    expect(adjacentKeyframe(PM, 1)).toBe(4);
  });
  it('does not wrap at endpoints, fall through an unanimated selection, or mutate keys', () => {
    const PM = pm(), before = JSON.stringify(PM.proj);
    PM.time = 6; expect(adjacentKeyframe(PM, 1)).toBeUndefined();
    expect(adjacentKeyframe(PM, -1)).toBe(5);
    PM.sel.chan = 'rotation'; expect(keyframeTimes(PM)).toEqual([]);
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
});
