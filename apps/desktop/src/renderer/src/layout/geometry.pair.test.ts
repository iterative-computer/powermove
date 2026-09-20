import { describe, expect, it } from 'vitest';

import { resolvePairResize, transferPanelHeights, visibleDockPlan } from './geometry';

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

describe('visibleDockPlan', () => {
  it('gives a dock whose panels are all sized one fluid panel so the column fills the window', () => {
    const workspace = {
      layout: {
        docks: [
          { id: 'left', size: 300, panels: [{ id: 'assets', size: 213 }, { id: 'random-hello-world', size: 166 }, { id: 'agent', size: 431 }] },
          { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 300 }] }
        ]
      }
    } as any;
    const plan = visibleDockPlan(workspace);
    const left = plan[0]!.specs;
    expect(left.map((spec) => !!spec.flex)).toEqual([false, true, false]);
    expect(left[2]!.size).toBe(431);
    // The center column already had a fluid viewer; nothing changes there.
    expect(plan[1]!.specs.map((spec) => !!spec.flex)).toEqual([true, false]);
  });
});

describe('visibleDockPlan with a panel this build does not have', () => {
  it('leaves it out of the column and gives another panel the fluid role', () => {
    // Jude's left column: an agent-authored curve editor from a different
    // machine held the dock's only flex, so nothing filled and the splitters
    // next to its empty slot had nothing to measure.
    const workspace = {
      layout: {
        docks: [
          { id: 'left', size: 300, panels: [{ id: 'assets', size: 347 }, { id: 'curve-editor', flex: true, size: 429 }, { id: 'agent', size: 574 }] }
        ]
      }
    } as any;
    const plan = visibleDockPlan(workspace, () => false, (id) => id !== 'curve-editor');
    const left = plan[0]!.specs;
    expect(left.map((spec) => spec.id)).toEqual(['assets', 'agent']);
    expect(left.map((spec) => !!spec.flex)).toEqual([true, false]);
    // The model keeps the panel for when its extension comes back.
    expect(workspace.layout.docks[0].panels.map((spec: any) => spec.id)).toEqual(['assets', 'curve-editor', 'agent']);
  });
});
