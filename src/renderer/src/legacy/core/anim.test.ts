import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './anim';

function animRegistry(): PMRegistry {
  let uid = 0;
  const PM: PMRegistry = {
    clamp: (value: any, min: any, max: any) => Math.max(min, Math.min(max, value)),
    Ease: {
      bezier: () => (value: any) => value,
      spring: (value: any) => value,
      handles: () => ({ eo: [.33, 0], ei: [.67, 1] }),
    },
    curComp: () => PM.proj,
    L: (id: any) => PM.proj.layers.find((layer: any) => layer.id === id) || null,
    uid: () => `k${++uid}`,
  };
  install(PM);
  return PM;
}

describe('legacy animation install', () => {
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

  it('falls back from malformed easing and steps discrete values', () => {
    const PM = animRegistry();
    const malformed = [{ t: 0, v: 0 }, { t: 1, v: 10, ei: [NaN] }];
    expect(PM.evalKfs(malformed, .5)).toBe(5);
    expect(Number.isNaN(PM.evalKfs(malformed, .5))).toBe(false);
    expect(PM.evalKfs([{ t: 0, v: 'a' }, { t: 1, v: 'b' }], .5)).toBe('a');
    expect(PM.evalKfs([{ t: 0, v: false }, { t: 1, v: true }], .999)).toBe(false);
    expect(PM.evalKfs([{ t: 0, v: false }, { t: 1, v: true }], 1)).toBe(true);
  });

  it('normalizes keyframes stably and keeps the newest duplicate', () => {
    const PM = animRegistry();
    const result = PM.normalizeKeyframes([
      { t: 1, v: 1, extra: 'old' },
      { t: -1, v: 8 },
      { t: 1, v: 2, eo: null, ei: [1], hold: 'yes', extra: 'new' },
      { t: 0, v: 'wrong-type' }, { t: '2', v: 3 },
    ], 0, 30);
    expect(result).toHaveLength(2);
    expect(result[0]).toMatchObject({ t: 1, v: 2, hold: false, extra: 'new' });
    expect(result[0].eo).toEqual([.33, 0]);
    expect(result[0].ei).toEqual([.67, 1]);
    expect(result[0].i).toBeTruthy();
    expect(result[1].t).toBe(2);
  });

  it('invalidates hierarchy memos across time, edits, and duplicate ids', () => {
    const PM = animRegistry();
    const prop = (v: any, kf: any[] = []) => ({ v, kf, expr: null });
    const layer = (id: string, x: number, opacity: number) => ({
      id, parent: null, from: 0, on: true, dur: 2,
      p: {
        'position.x': prop(x, [{ t: 0, v: x, eo: [0, 0] }, { t: 1, v: x + 10, ei: [1, 1] }]),
        'position.y': prop(0), 'anchor.x': prop(0), 'anchor.y': prop(0),
        'scale.x': prop(100), 'scale.y': prop(100), rotation: prop(0), skew: prop(0),
        opacity: prop(opacity, [{ t: 0, v: opacity, eo: [0, 0] }, { t: 1, v: opacity / 2, ei: [1, 1] }]),
      },
    });
    const first = layer('duplicate', 0, 100);
    const second = layer('duplicate', 50, 80);
    PM.proj = { fps: 30, dur: 2, layers: [first, second] };

    PM.beginEval(0);
    expect(PM.worldMatrix(first, 0)[4]).toBe(0);
    expect(PM.worldOpacity(first, 0)).toBe(1);
    expect(PM.worldMatrix(second, 0)[4]).toBe(50);
    expect(PM.worldOpacity(second, 0)).toBe(.8);
    PM.beginEval(1);
    expect(PM.worldMatrix(first, 1)[4]).toBe(10);
    expect(PM.worldOpacity(first, 1)).toBe(.5);
    first.p['position.x'].v = 99;
    first.p['position.x'].kf = [];
    PM.touch();
    PM.beginEval(1);
    expect(PM.worldMatrix(first, 1)[4]).toBe(99);
  });

  it('includes only a layer ending at the current composition endpoint', () => {
    const PM = animRegistry();
    PM.proj = { dur: 10, layers: [] };
    expect(PM.active({ on: true, from: 0, dur: 10 }, 10)).toBe(true);
    expect(PM.active({ on: true, from: 0, dur: 5 }, 5)).toBe(false);
    expect(PM.active({ on: true, from: 5, dur: 5 }, 10)).toBe(true);
    expect(PM.active({ on: true, from: NaN, dur: 5 }, 1)).toBe(false);
    expect(PM.active({ on: true, from: 0, dur: Infinity }, 1)).toBe(false);
  });

  it('exposes transition channels through the shared animation registry', () => {
    const PM = animRegistry();
    PM.CH = {};
    const angle = { v: 0, kf: [], expr: null };
    const layer: any = { p: {}, fx: [], masks: [], d: {}, transitionIn: { p: { angle } } };

    expect(PM.allProps(layer)).toContainEqual({
      key: 'transitionIn.p.angle', prop: angle, label: 'angle', group: 'Transition in',
    });
    expect(PM.findProp(layer, 'transitionIn.p.angle')).toBe(angle);
  });
});
