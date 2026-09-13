import { describe, it, expect, vi } from 'vitest';
import { adjacentKeyframe, keyframeTimes } from './keyframe-navigation';
import { fakePowermoveAPI } from './fake-api.test-helper';

describe('keyframe navigation', () => {
  const property = (key: string, times: number[]) => ({ key, prop: { kf: times.map(t => ({ t })) } });
  const setup = () => {
    const harness = fakePowermoveAPI(vi);
    harness.state.time = 2;
    harness.state.project = { dur: 8, layers: [
      { id: 'a', from: 1, shy: false, threeD: false, props: [property('position.x', [0, 2]), property('scale.x', [1]), property('scale.y', [3])] },
      { id: 'b', from: 0, shy: false, threeD: false, props: [property('opacity', [1, 5, NaN, 20])] },
    ] };
    harness.api.anim.allProps = ((layer: any) => layer.props) as typeof harness.api.anim.allProps;
    return harness;
  };

  it('uses real keys, offsets, finite composition times and both Scale channels', () => {
    const { api, state } = setup();
    expect(keyframeTimes(api)).toEqual([1, 2, 3, 4, 5]);
    state.selection.layers = ['a']; state.selection.chan = 'scale';
    expect(keyframeTimes(api)).toEqual([2, 4]);
    expect(adjacentKeyframe(api, -1)).toBeUndefined();
    expect(adjacentKeyframe(api, 1)).toBe(4);
  });

  it('does not wrap at endpoints, fall through an unanimated selection, or mutate keys', () => {
    const { api, state } = setup(), before = JSON.stringify(state.project);
    state.time = 6; expect(adjacentKeyframe(api, 1)).toBeUndefined();
    expect(adjacentKeyframe(api, -1)).toBe(5);
    state.selection.chan = 'rotation'; expect(keyframeTimes(api)).toEqual([]);
    expect(JSON.stringify(state.project)).toBe(before);
  });
});
