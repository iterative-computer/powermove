import { describe, expect, it } from 'vitest';

import { selectionAfterKeyGesture } from './graph-selection';
import { keyframeContextEntries, resolveKeyframeGroupSnap } from './timeline';
import { goToWorkAreaBoundary, timelineEventTimes } from '../../renderer/src/legacy/ui/shortcuts';

describe('professional timeline gestures', () => {
  it('snaps a multi-keyframe group by whichever selected key reaches a target', () => {
    expect(resolveKeyframeGroupSnap([1, 2.5], .46, [3, 5], .05)).toEqual({
      delta: .5,
      lock: { anchor: 2.5, target: 3 },
    });
  });

  it('holds a group snap until the anchored key clears the release radius', () => {
    const lock = { anchor: 2.5, target: 3 };
    expect(resolveKeyframeGroupSnap([1, 2.5], .56, [3, 3.04], .05, lock, .08))
      .toEqual({ delta: .5, lock });
    expect(resolveKeyframeGroupSnap([1, 2.5], .7, [3, 3.04], .05, lock, .08))
      .toEqual({ delta: .7, lock: null });
  });

  it('distinguishes Shift-click toggle from Shift-drag movement', () => {
    expect(selectionAfterKeyGesture(['a', 'b'], ['b'], true, false)).toEqual(['a']);
    expect(selectionAfterKeyGesture(['a', 'b'], ['b'], true, true)).toEqual(['a', 'b']);
    expect(selectionAfterKeyGesture(['a'], ['b'], true, true)).toEqual(['a', 'b']);
    expect(selectionAfterKeyGesture(['a'], ['b'], false, true)).toEqual(['b']);
  });

  it('targets the full selection only when the context-clicked key is selected', () => {
    const first = { key: { i: 'first' }, prop: { id: 'opacity' } };
    const second = { key: { i: 'second' }, prop: { id: 'scale' } };
    const outside = { key: { i: 'outside' }, prop: { id: 'rotation' } };

    expect(keyframeContextEntries([first], ['first', 'second'], [first, second])).toEqual([first, second]);
    expect(keyframeContextEntries([outside], ['first', 'second'], [first, second])).toEqual([outside]);
  });

  it('jumps to clamped work-area boundaries without editing the work area', () => {
    const PM: any = {
      proj: { dur: 10, work: [-1, 12] },
      time: 4,
      setTime(time: number) { this.time = time; },
    };

    expect(goToWorkAreaBoundary(PM, 'in')).toBe(0);
    expect(goToWorkAreaBoundary(PM, 'out')).toBe(10);
    expect(PM.proj.work).toEqual([-1, 12]);
  });

  it('keeps selected-event navigation scoped to selected layer events', () => {
    const key = { t: 1 };
    const layer = { id: 'selected', from: 2, dur: 5, prop: { kf: [key] } };
    const PM: any = {
      proj: { dur: 10, work: [1, 9], markers: [{ t: 4 }], layers: [layer] },
      sel: { layers: [layer.id] },
      allProps(target: any) { return [{ prop: target.prop }]; },
    };

    expect(timelineEventTimes(PM, true)).toEqual([2, 3, 7]);
  });
});
