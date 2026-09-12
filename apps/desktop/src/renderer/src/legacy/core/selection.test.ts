import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './selection';

function selectionRegistry(): { PM: PMRegistry; events: string[] } {
  const events: string[] = [];
  const PM: PMRegistry = {
    projGeneration: 0,
    proj: null,
    sel: { layers: [], keys: [], chan: null },
    time: 9,
    exprCache: new Map([['value + 1', {}]]),
    rasterClears: 0,
    rasterClear() { PM.rasterClears++; },
    touch() {},
    invalidate() {},
    bus: { emit(name: string) { events.push(name); } },
  };
  install(PM);
  return { PM, events };
}

describe('legacy selection install', () => {
  it('prunes stale selections, clamps time, and invalidates in legacy event order', () => {
    const { PM, events } = selectionRegistry();
    const first = { i: 'key-a', t: 0, v: 100 };
    const project = {
      dur: 2,
      layers: [{ id: 'layer-a', p: { opacity: { kf: [first] } }, fx: [], masks: [], d: {} }],
      comps: {},
    };

    PM.replaceProject(project, {
      selection: { layers: ['layer-a', 'dead'], keys: [first, 'dead'], chan: 'opacity' },
    });

    expect(PM.sel.layers).toEqual(['layer-a']);
    expect(PM.sel.keys).toEqual([]);
    expect(PM.time).toBe(2);
    expect(PM.exprCache.size).toBe(0);
    expect(PM.rasterClears).toBe(1);
    expect(PM.projGeneration).toBe(1);
    expect(events).toEqual(['layers', 'sel', 'assets', 'project']);
  });

  it('resolves nested live keyframe ids once and in selection order', () => {
    const { PM } = selectionRegistry();
    const first = { i: 'key-a' };
    const nested = { i: 'key-b' };
    PM.proj = {
      layers: [{ p: { opacity: { kf: [first] } } }],
      comps: { child: { layers: [{ p: {}, fx: [{ p: { amount: { kf: [nested] } } }] }] } },
    };
    PM.sel.keys = ['key-b', 'key-a', 'key-b', 'dead'];

    expect(PM.resolveSelectedKeys()).toEqual([nested, first]);
  });

  it('resolves transition keyframes as first-class animation channels', () => {
    const { PM } = selectionRegistry();
    const transition = { i: 'transition-key' };
    PM.proj = {
      layers: [{
        p: {}, fx: [], masks: [], d: {},
        transitionIn: { p: { angle: { kf: [transition] } } },
      }],
      comps: {},
    };
    PM.sel.keys = ['transition-key'];

    expect(PM.resolveSelectedKeys()).toEqual([transition]);
  });
});
