import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './anim';
import { makePM } from '../__tests__/make-pm';

function animRegistry(): PMRegistry {
  const PM: PMRegistry = {
    clamp: (value: any, min: any, max: any) => Math.max(min, Math.min(max, value)),
    Ease: {
      bezier: () => (value: any) => value,
      spring: (value: any) => value,
      handles: () => ({ eo: [.33, 0], ei: [.67, 1] }),
    },
    curComp: () => PM.proj,
    L: (id: any) => PM.proj.layers.find((layer: any) => layer.id === id) || null,
  };
  install(PM);
  return PM;
}

describe('legacy animation install', () => {
  it('defaults every new-key path to linear and preserves explicit easing', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject();
    const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
    const check = (key: any) => expect(key).toMatchObject({ eo: [0, 0], ei: [1, 1] });
    check(PM.KF(0, 10));
    PM.toggleStopwatch(layer, 'opacity', 0);
    check(layer.p.opacity.kf[0]);
    PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 50, time: 1 });
    check(layer.p.opacity.kf[1]);
    PM.animate(layer, 'rotation', [{ t: 0, v: 0 }, { t: 1, v: 90 }]);
    layer.p.rotation.kf.forEach(check);
    PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path: 'position.x', keyframes: [{ time: 0, value: 0 }, { time: 1, value: 100 }] });
    layer.p['position.x'].kf.forEach(check);
    expect(PM.KF(0, 10, 'power').eo).toEqual([.62, .05]);
  });
  it('detects self, descendant, unrelated, existing, and empty parents', () => {
    const PM = animRegistry();
    const mk = (id: any, parent: any) => ({ id, parent });
    PM.proj = { layers: [mk('a', null), mk('b', 'a'), mk('c', 'b'), mk('z', null)] };

    expect(PM.wouldCycle(PM.proj.layers[0], 'a')).toBe(true);
    expect(PM.wouldCycle(PM.proj.layers[0], 'c')).toBe(true);
    expect(PM.wouldCycle(PM.proj.layers[0], 'z')).toBe(false);
    expect(PM.wouldCycle(PM.proj.layers[2], 'b')).toBe(false);
    expect(PM.wouldCycle(PM.proj.layers[0], null)).toBe(false);
  });

  it('clamps keyframe evaluation to the legacy endpoints', () => {
    const PM = animRegistry();
    const keyframes = [
      { t: .2, v: 10, eo: [0, 0], ei: [1, 1] },
      { t: .8, v: 30, eo: [0, 0], ei: [1, 1] },
    ];

    expect(PM.evalKfs(keyframes, -1)).toBe(10);
    expect(PM.evalKfs(keyframes, 1.5)).toBe(30);
    expect(PM.evalKfs(keyframes, .5)).toBe(20);
  });
});
