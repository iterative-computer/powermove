import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { EngineCanvasHold, holdsCommittedCanvas, type CommittedCanvas, type HoldInput } from './canvas-hold';
import { todaysCanvas } from './view-params';
import type { ViewerCanvas } from './protocol';

const project = { w: 640, h: 360 };

/** Today's canvas at 50% on a 2x display: the whole composition, 640 × 360. */
const whole: CommittedCanvas = { render: { width: 640, height: 360 }, viewport: null, project, compWidth: 640, compHeight: 360, version: 7, time: 0.5, quality: 1 };
/** A crop at 237%: composition (100, 40) to (420, 220). */
const crop = { x: 100, y: 40, width: 320, height: 180 };
const cropped: CommittedCanvas = { ...whole, render: { width: 1517, height: 853 }, viewport: crop };

/** The canvas the engine drew a committed canvas at, as the viewer sent it. */
const sent = (c: CommittedCanvas): ViewerCanvas => todaysCanvas(c.compWidth, c.compHeight, c.render, c.viewport, { x: 10, y: 20 }, 1)!;

const input = (over: Partial<HoldInput> = {}): HoldInput => ({
  gesture: true, now: 1000, gestureUntil: 1100, playing: false,
  next: { ...whole, render: { width: 512, height: 288 } },
  visible: { x: 0, y: 0, right: 640, bottom: 360 },
  onScreen: { canvas: sent(whole), current: true },
  ...over,
});

describe('the engine canvas hold (today’s canRetainNavigation)', () => {
  it('holds the committed canvas inside a gesture while the frame on screen is current and drawn at it', () => {
    expect(holdsCommittedCanvas(whole, input())).toBe(true);
  });

  it('releases outside a gesture, after it settles, while playing, or with nothing committed', () => {
    expect(holdsCommittedCanvas(whole, input({ gesture: false }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ now: 1100 }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ playing: true }))).toBe(false);
    expect(holdsCommittedCanvas(null, input())).toBe(false);
  });

  it('releases on a version, time, quality, project or composition size change', () => {
    const next = input().next;
    expect(holdsCommittedCanvas(whole, input({ next: { ...next, version: 8 } }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ next: { ...next, time: 0.6 } }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ next: { ...next, quality: 0.5 } }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ next: { ...next, project: { ...project } } }))).toBe(false);
    // The same project object resized (today's presentationIsCurrent reads project.w/h).
    expect(holdsCommittedCanvas(whole, input({ next: { ...next, compWidth: 1280 } }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ next: { ...next, compHeight: 720 } }))).toBe(false);
  });

  it('releases until the engine has presented a current frame at the committed canvas', () => {
    expect(holdsCommittedCanvas(whole, input({ onScreen: null }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ onScreen: { canvas: sent(whole), current: false } }))).toBe(false);
    // A frame of an earlier canvas is on screen.
    expect(holdsCommittedCanvas(whole, input({ onScreen: { canvas: sent({ ...whole, render: { width: 320, height: 180 } }), current: true } }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ onScreen: { canvas: sent(cropped), current: true } }))).toBe(false);
    expect(holdsCommittedCanvas(whole, input({ onScreen: { canvas: undefined, current: true } }))).toBe(false);
  });

  it('holds a crop while the visible area stays inside it, and releases when it leaves', () => {
    const at = (visible: HoldInput['visible']) => input({ next: { ...cropped, viewport: { ...crop, x: 120 } }, visible, onScreen: { canvas: sent(cropped), current: true } });
    expect(holdsCommittedCanvas(cropped, at({ x: 100, y: 40, right: 420, bottom: 220 }))).toBe(true);
    expect(holdsCommittedCanvas(cropped, at({ x: 150, y: 60, right: 400, bottom: 200 }))).toBe(true);
    expect(holdsCommittedCanvas(cropped, at({ x: 99, y: 60, right: 400, bottom: 200 }))).toBe(false);
    expect(holdsCommittedCanvas(cropped, at({ x: 150, y: 39, right: 400, bottom: 200 }))).toBe(false);
    expect(holdsCommittedCanvas(cropped, at({ x: 150, y: 60, right: 421, bottom: 200 }))).toBe(false);
    expect(holdsCommittedCanvas(cropped, at({ x: 150, y: 60, right: 400, bottom: 221 }))).toBe(false);
  });
});

describe('EngineCanvasHold', () => {
  let now = 0, until = 0;
  const relayout = vi.fn();
  const make = () => new EngineCanvasHold({
    setTimeout: (fn, ms) => setTimeout(fn, ms) as unknown as number,
    clearTimeout: (id) => clearTimeout(id),
    now: () => now,
    gestureUntil: () => until,
    relayout,
  });
  beforeEach(() => { vi.useFakeTimers(); relayout.mockReset(); now = 1000; until = 1100; });
  afterEach(() => vi.useRealTimers());

  /** Advances the fake clock and the hold's clock together. */
  const advance = (ms: number) => { now += ms; vi.advanceTimersByTime(ms); };

  it('commits outside gestures, holds inside one, and lays out again once the gesture settles', () => {
    const hold = make();
    expect(hold.layout(input({ gesture: false, next: whole }))).toBe(whole);
    expect(hold.committed).toBe(whole);
    // Zooming: today's canvas would change; the committed one is kept.
    expect(hold.layout(input())).toBe(whole);
    expect(hold.settling).toBe(true);
    // A later event of the gesture moves its end: no settle at the old end.
    advance(50); until = now + 100;
    expect(hold.layout(input({ now }))).toBe(whole);
    advance(60);
    expect(relayout).not.toHaveBeenCalled();
    advance(41);
    expect(relayout).toHaveBeenCalledTimes(1);
    expect(hold.settling).toBe(false);
    // The settle's layout is not a gesture: today's canvas is committed.
    const settled = input({ gesture: false, now });
    expect(hold.layout(settled)).toBe(settled.next);
    expect(hold.committed).toBe(settled.next);
  });

  it('commits the new canvas and drops the settle when the hold releases mid-gesture', () => {
    const hold = make();
    hold.layout(input({ gesture: false, next: whole }));
    hold.layout(input());
    expect(hold.settling).toBe(true);
    const next = { ...input().next, version: 8 };
    expect(hold.layout(input({ next }))).toBe(next);
    expect(hold.settling).toBe(false);
    advance(500);
    expect(relayout).not.toHaveBeenCalled();
  });

  it('does not hold before the engine presented the committed canvas', () => {
    const hold = make();
    hold.layout(input({ gesture: false, next: whole }));
    const unpresented = input({ onScreen: null });
    expect(hold.layout(unpresented)).toBe(unpresented.next);
    expect(hold.settling).toBe(false);
  });

  it('disposes its settle', () => {
    const hold = make();
    hold.layout(input({ gesture: false, next: whole }));
    hold.layout(input());
    hold.dispose();
    advance(500);
    expect(relayout).not.toHaveBeenCalled();
    expect(hold.committed).toBeNull();
  });
});
