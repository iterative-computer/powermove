import { describe, expect, it } from 'vitest';

import { resolvePairResize, transferPanelHeights } from './geometry';

describe('horizontal splitter pair resolution', () => {
  it('resizes the sized neighbour when exactly one panel flexes', () => {
    expect(resolvePairResize(true, false)).toEqual({ mode: 'single', target: 'after', sign: -1 });
    expect(resolvePairResize(false, true)).toEqual({ mode: 'single', target: 'before', sign: 1 });
  });

  it('transfers height between two sized panels instead of converting one to flex', () => {
    // viewer(flex) · timeline(300) · shader(320): the timeline/shader splitter
    // must move height between those two, never let the viewer absorb it.
    expect(resolvePairResize(false, false)).toEqual({ mode: 'transfer' });
  });

  it('pins the after panel when both flex', () => {
    expect(resolvePairResize(true, true)).toEqual({ mode: 'single', target: 'after', sign: -1 });
  });
});

describe('transferPanelHeights', () => {
  it('keeps the pair total invariant', () => {
    expect(transferPanelHeights(300, 320, -100)).toEqual({ before: 200, after: 420 });
    expect(transferPanelHeights(300, 320, 50)).toEqual({ before: 350, after: 270 });
  });

  it('clamps to both minimums', () => {
    expect(transferPanelHeights(300, 320, -900, 88, 100)).toEqual({ before: 88, after: 532 });
    expect(transferPanelHeights(300, 320, 900, 88, 100)).toEqual({ before: 520, after: 100 });
  });
});
