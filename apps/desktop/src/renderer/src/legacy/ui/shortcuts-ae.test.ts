import { beforeEach, describe, expect, it } from 'vitest';

import { makePM } from '../__tests__/make-pm';
import {
  editSelectedLayerTiming,
  goToSelectedLayerBoundary,
  goToTimelineEvent,
  nudgeKeyframes,
  selectAdjacentLayer,
  setLayerLocks,
  timelineEventTimes,
  toggleLayerControls,
} from './shortcuts';

function runtime() {
  const PM: any = makePM(
    'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history',
    'core/editing', 'core/capabilities', 'core/media', 'ui/shortcuts',
  );
  PM.proj = PM.mkProject({ w: 1920, h: 1080, fps: 10, dur: 10 });
  PM.time = 0;
  PM.setTime = (time: number) => { PM.time = time; };
  return PM;
}

function add(PM: any, id: string, from: number, dur: number, type = 'solid') {
  const layer = PM.mkLayer(type, { name: id, from, dur });
  layer.id = id;
  layer.from = from;
  layer.dur = dur;
  PM.proj.layers.push(layer);
  return layer;
}

describe('After Effects shortcut fundamentals', () => {
  let PM: any;

  beforeEach(() => { PM = runtime(); });

  it('navigates work-area ends, markers, layer boundaries, and keyframes with J/K', () => {
    const layer = add(PM, 'title', 2, 5);
    PM.setKey(layer, 'opacity', 3, 50);
    PM.proj.work = [1, 9];
    PM.proj.markers = [{ t: 4, name: 'Beat' }];

    expect(timelineEventTimes(PM)).toEqual([0, 1, 2, 3, 4, 7, 9, 10]);
    PM.time = 3.25;
    expect(goToTimelineEvent(PM, 1)).toBe(4);
    expect(PM.time).toBe(4);
    expect(goToTimelineEvent(PM, -1)).toBe(3);
    expect(PM.time).toBe(3);
  });

  it('uses I/O for selected layer boundaries and brackets for moving and trimming', () => {
    const first = add(PM, 'first', 2, 4);
    const second = add(PM, 'second', 3, 5);
    PM.selectLayers([first.id, second.id]);
    PM.hist.clear(); // Start the command history after selecting its targets.

    expect(goToSelectedLayerBoundary(PM, 'in')).toBe(2);
    expect(goToSelectedLayerBoundary(PM, 'out')).toBe(8);

    PM.time = 5;
    expect(editSelectedLayerTiming(PM, 'moveIn')).toMatchObject({ ok: true });
    expect(PM.proj.layers.map((layer: any) => [layer.from, layer.dur])).toEqual([[5, 4], [5, 5]]);
    expect(PM.hist.list()).toEqual(['Move layer In point']);
    expect(PM.hist.undo()).toBe(true);

    PM.time = 4;
    expect(editSelectedLayerTiming(PM, 'trimIn')).toMatchObject({ ok: true });
    expect(PM.proj.layers.map((layer: any) => [layer.from, layer.dur])).toEqual([[4, 2], [4, 4]]);
    expect(PM.hist.undo()).toBe(true);

    PM.time = 5;
    expect(editSelectedLayerTiming(PM, 'trimOut')).toMatchObject({ ok: true });
    expect(PM.proj.layers.map((layer: any) => [layer.from, layer.dur])).toEqual([[2, 3], [3, 2]]);
  });

  it('moves selected keyframes by exact frames and resolves destination collisions', () => {
    const layer = add(PM, 'animated', 2, 5);
    const moving = PM.setKey(layer, 'opacity', 3, 25);
    PM.setKey(layer, 'opacity', 3.1, 50);
    PM.sel.keys = [moving.i];

    expect(nudgeKeyframes(PM, 1)).not.toBe(false);
    expect(layer.p.opacity.kf.map((key: any) => [key.t, key.v])).toEqual([[1.1, 25]]);
    expect(PM.hist.list()).toEqual(['Move keyframes']);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(layer.id).p.opacity.kf.map((key: any) => [key.t, key.v])).toEqual([[1, 25], [1.1, 50]]);
  });

  it('hides layer controls without hiding or mutating any layer', () => {
    const layer = add(PM, 'visible', 0, 10);
    const viewer: { showControls?: boolean } = {};
    PM.Kernel.services.register('viewer', viewer);

    expect(toggleLayerControls(PM)).toBe(false);
    expect(layer.on).toBe(true);
    expect(PM.hist.list()).toEqual([]);
    expect(toggleLayerControls(PM)).toBe(true);
    expect(layer.on).toBe(true);
  });

  it('selects adjacent layers and uses AE lock shortcuts without changing content', () => {
    const first = add(PM, 'first', 0, 10);
    const second = add(PM, 'second', 0, 10);
    const third = add(PM, 'third', 0, 10);
    PM.selectLayers([second.id]);

    expect(selectAdjacentLayer(PM, -1)).toBe(first);
    expect(PM.sel.layers).toEqual([first.id]);
    expect(selectAdjacentLayer(PM, 1, true)).toBe(second);
    expect(PM.sel.layers).toEqual([first.id, second.id]);

    PM.hist.clear(); // The navigation above already verified its selection changes.
    expect(setLayerLocks(PM, true)).toMatchObject({ ok: true });
    expect([first.lock, second.lock, third.lock]).toEqual([true, true, false]);
    expect(PM.hist.list()).toEqual(['Lock layers']);

    expect(setLayerLocks(PM, false, true)).toMatchObject({ ok: true });
    expect([first.lock, second.lock, third.lock]).toEqual([false, false, false]);
    expect(PM.hist.list()).toEqual(['Lock layers', 'Unlock layers']);
  });
});
