import { describe, expect, it } from 'vitest';

import { decisionOffsetUs, JitterBuffer, type Queued } from './jitter-buffer';
import { mediaTimeAt, TICKS_PER_SECOND, type ClockSample, type FrameDescriptor } from './protocol';

const F = TICKS_PER_SECOND / 60;

function desc(seq: number, pts: number, duration: number | null = F, generation = 1): FrameDescriptor {
  return {
    poolIndex: seq % 8,
    poolGeneration: generation,
    seq,
    pts,
    revision: 1,
    colorSpace: 'srgb',
    format: 'bgra8Unorm',
    quality: { resolution: 'full', representation: 'original', draft3d: false, cached: false },
    overlay: null,
    depthId: null,
    viewSeq: 1,
    duration
  };
}

const item = (d: FrameDescriptor): Queued<string> => ({ desc: d, payload: `f${d.seq}`, arrivedMs: 0 });

function clock(partial: Partial<ClockSample> = {}): ClockSample {
  return {
    seq: 1,
    epoch: 2,
    state: 'playing',
    hostTime: 1_000_000,
    mediaTime: 0,
    rate: { num: 1, den: 1 },
    outputLatencyUs: 0,
    sampleRate: 48_000,
    loopRange: null,
    ...partial
  };
}

describe('mediaTimeAt', () => {
  it('follows the line and wraps in a loop', () => {
    expect(mediaTimeAt(clock(), 1_500_000)).toBe(TICKS_PER_SECOND / 2);
    expect(mediaTimeAt(clock({ state: 'stopped', rate: { num: 0, den: 1 }, mediaTime: 7 }), 9e9)).toBe(7);
    const looped = clock({ loopRange: { start: 0, end: 60 * F } });
    expect(mediaTimeAt(looped, 1_000_000 + 1_500_000)).toBe(30 * F);
  });
});

describe('JitterBuffer', () => {
  it('draws the newest frame that arrived after its interval when nothing on time is there, within the grace, and never goes back', () => {
    const buffer = new JitterBuffer<string>(250_000);
    const c = clock();
    // Frames 0 to 2 arrive 120 ms after the line started (their intervals
    // ended 70 to 103 ms ago): the newest is drawn, the others dropped.
    for (let k = 0; k < 3; k++) buffer.push(item(desc(k + 1, k * F)));
    const first = buffer.pick(1_120_000, c);
    expect(first.show?.desc.seq).toBe(3);
    expect(first.late).toBe(true);
    expect(first.drop.map((d) => [d.item.desc.seq, d.reason])).toEqual([[1, 'late'], [2, 'late']]);
    // An older frame arriving later is never drawn over it.
    buffer.push(item(desc(9, F)));
    expect(buffer.pick(1_130_000, c).show).toBeNull();
    // Past the grace: dropped, the picture stays.
    buffer.push(item(desc(10, 3 * F)));
    const old = buffer.pick(1_050_000 + 3 * 16_667 + 300_000, c);
    expect(old.show).toBeNull();
    expect(old.drop.map((d) => d.item.desc.seq)).toEqual([10]);
    // A frame on time still wins over a late one.
    buffer.push(item(desc(11, 30 * F)));
    buffer.push(item(desc(12, 31 * F)));
    const onTime = buffer.pick(1_000_000 + 31 * 16_667 + 5_000, c);
    expect(onTime.show?.desc.seq).toBe(12);
    expect(onTime.late).toBeUndefined();
    // Without a grace (the default), late frames are dropped as before.
    const strict = new JitterBuffer<string>();
    strict.push(item(desc(1, 0)));
    expect(strict.pick(1_120_000, c).show).toBeNull();
  });

  it('shows the frame whose interval contains the predicted display time', () => {
    const buffer = new JitterBuffer<string>();
    for (let k = 0; k < 6; k++) buffer.push(item(desc(k + 1, k * F)));
    const c = clock();
    // 25 ms after the line started: frame 1 (16.7 to 33.3 ms) is on screen.
    const first = buffer.pick(1_025_000, c);
    expect(first.show?.desc.seq).toBe(2);
    expect(first.drop.map((d) => [d.item.desc.seq, d.reason])).toEqual([[1, 'late']]);
    // Still frame 1's interval: nothing new.
    expect(buffer.pick(1_030_000, c).show).toBeNull();
    // 70 ms: frames 2 and 3 passed, frame 4 (66.7 to 83.3 ms) shows.
    const later = buffer.pick(1_070_000, c);
    expect(later.show?.desc.seq).toBe(5);
    expect(later.drop.map((d) => d.item.desc.seq)).toEqual([3, 4]);
  });

  it('replaces a queued frame with a newer revision of the same frame, in place', () => {
    const buffer = new JitterBuffer<string>();
    const at = (seq: number, k: number, revision: number) => item({ ...desc(seq, k * F), revision });
    for (let k = 0; k < 4; k++) buffer.push(at(k + 1, k, 1));
    // An edit: frames 1 to 3 rendered again at revision 2.
    expect(buffer.push(at(5, 1, 2)).map((q) => q.desc.seq)).toEqual([2]);
    expect(buffer.push(at(6, 2, 2)).map((q) => q.desc.seq)).toEqual([3]);
    expect(buffer.size).toBe(4);
    // An older revision arriving late never replaces a newer one.
    expect(buffer.push(at(7, 2, 1)).map((q) => q.desc.seq)).toEqual([7]);
    // The same revision twice (a loop's next lap) is kept as before.
    expect(buffer.push(at(8, 3, 1))).toEqual([]);
    const c = clock();
    expect(buffer.pick(1_025_000, c).show?.desc.seq).toBe(5);
    expect(buffer.pick(1_040_000, c).show?.desc.seq).toBe(6);
    expect(buffer.pick(1_055_000, c).show?.desc.seq).toBe(4);
  });

  it('keeps frames that are not due yet', () => {
    const buffer = new JitterBuffer<string>();
    buffer.push(item(desc(1, 10 * F)));
    expect(buffer.pick(1_000_000, clock())).toEqual({ show: null, drop: [] });
    expect(buffer.size).toBe(1);
  });

  it('drops timed frames and shows the newest immediate frame when stopped', () => {
    const buffer = new JitterBuffer<string>();
    buffer.push(item(desc(1, 0)));
    expect(buffer.push(item(desc(2, 0, null)))).toEqual([]);
    const superseded = buffer.push(item(desc(3, 0, null)));
    expect(superseded.map((i) => i.desc.seq)).toEqual([2]);
    const stopped = clock({ state: 'stopped', rate: { num: 0, den: 1 } });
    const pick = buffer.pick(0, stopped);
    expect(pick.show?.desc.seq).toBe(3);
    expect(pick.drop.map((d) => [d.item.desc.seq, d.reason])).toEqual([[1, 'stopped']]);
    expect(buffer.size).toBe(0);
  });

  it('crosses a loop wrap without dropping the next lap', () => {
    const buffer = new JitterBuffer<string>();
    const c = clock({ loopRange: { start: 0, end: 60 * F } });
    buffer.push(item(desc(1, 59 * F)));
    buffer.push(item(desc(2, 0)));
    // 59.5 frames in: frame 59 shows, the next lap's frame 0 waits.
    const at = 1_000_000 + (59.5 * 1e6) / 60;
    expect(buffer.pick(at, c).show?.desc.seq).toBe(1);
    expect(buffer.size).toBe(1);
    expect(buffer.pick(at + 1e6 / 60, c).show?.desc.seq).toBe(2);
  });

  it('plays in reverse', () => {
    const buffer = new JitterBuffer<string>();
    const c = clock({ mediaTime: 60 * F, rate: { num: -1, den: 1 } });
    buffer.push(item(desc(1, 59 * F)));
    buffer.push(item(desc(2, 58 * F)));
    buffer.push(item(desc(3, 57 * F)));
    // 20 ms after the start in reverse: media at 58.8 frames: frame 58 shows, 59 is late.
    const pick = buffer.pick(1_020_000, c);
    expect(pick.show?.desc.seq).toBe(2);
    expect(pick.drop.map((d) => d.item.desc.seq)).toEqual([1]);
  });
});

describe('decisionOffsetUs', () => {
  it('puts frame boundaries halfway between decisions when frames last whole refreshes', () => {
    const period = 1e6 / 120;
    const c = clock({ hostTime: 1_000_000, mediaTime: 0 });
    for (const decision of [1_000_000, 1_001_000, 1_004_166, 1_007_000]) {
      const offset = decisionOffsetUs(period, decision, c, F);
      expect(Math.abs(offset)).toBeLessThanOrEqual(period / 2);
      // Every boundary (1_000_000 + n × 16_667) is half a period from a decision.
      const phase = ((1_000_000 - (decision + offset)) % period + period) % period;
      expect(phase).toBeCloseTo(period / 2, 3);
    }
  });

  it('leaves decisions alone when frames do not last whole refreshes', () => {
    // 25 fps on 120 Hz: 4.8 refreshes per frame.
    expect(decisionOffsetUs(1e6 / 120, 1_000_000, clock(), TICKS_PER_SECOND / 25)).toBe(0);
    expect(decisionOffsetUs(1e6 / 120, 1_000_000, null, F)).toBe(0);
  });
});
