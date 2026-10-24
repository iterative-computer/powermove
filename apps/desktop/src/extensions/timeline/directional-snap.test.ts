import { describe, expect, it } from 'vitest';
import {
  createDirectionalSnapper,
  type DirectionalSnapAnchor,
  type DirectionalSnapLock,
  type DirectionalSnapTarget,
} from './directional-snap';

// Exhaustive reference: preserves the pre-index search and source-order ties.
function exhaustiveSnapper(initialDelta = 0) {
  let previous = Number.isFinite(initialDelta) ? initialDelta : 0;
  let lock: DirectionalSnapLock | null = null;
  const epsilon = 1e-9;
  return {
    resolve(anchors: DirectionalSnapAnchor[], requested: number, targets: DirectionalSnapTarget[], tolerance: number, enabled = true) {
      const raw = Number.isFinite(requested) ? requested : previous, before = previous;
      previous = raw;
      const travel = raw - before;
      const direction = Math.abs(travel) <= epsilon ? 0 : travel > 0 ? 1 : -1;
      const radius = Math.max(0, Number.isFinite(tolerance) ? tolerance : 0);
      if (!enabled) { lock = null; return { delta: raw, lock }; }
      const released = lock;
      if (lock) {
        const anchor = anchors[lock.anchor];
        const exists = anchor && targets.some(target => target.time === lock!.target && (!target.edge || target.edge === anchor.edge));
        if (exists && (!direction || direction === lock.direction) && Math.abs(anchor.time + raw - lock.target) <= radius + epsilon) {
          return { delta: lock.target - anchor.time, lock };
        }
        lock = null;
      }
      if (!direction) return { delta: raw, lock };
      let bestDistance = Infinity, bestWeight = -Infinity;
      for (const [index, anchor] of anchors.entries()) {
        if (!Number.isFinite(anchor.time)) continue;
        for (const target of targets) {
          if (!Number.isFinite(target.time) || (target.edge && target.edge !== anchor.edge)) continue;
          if (released?.anchor === index && released.target === target.time) continue;
          const at = target.time - anchor.time;
          const crossed = direction > 0 ? at > before + epsilon && at <= raw + epsilon : at < before - epsilon && at >= raw - epsilon;
          const distance = Math.abs(raw - at);
          if (!crossed || distance > radius + epsilon) continue;
          const weight = Number.isFinite(target.weight) ? target.weight! : 1;
          if (distance < bestDistance - epsilon || (Math.abs(distance - bestDistance) <= epsilon && weight > bestWeight)) {
            bestDistance = distance; bestWeight = weight;
            lock = { anchor: index, target: target.time, direction };
          }
        }
      }
      return lock ? { delta: lock.target - anchors[lock.anchor]!.time, lock } : { delta: raw, lock: null };
    },
  };
}

describe('directional timeline latching', () => {
  const anchors = [{ time: 0 }];
  const targets = [{ time: 2 }];
  it('does not attract a pointer near an edge, and acquires only when crossing it', () => {
    const snap = createDirectionalSnapper(1);
    expect(snap.resolve(anchors, 1.98, targets, .15)).toEqual({ delta: 1.98, lock: null });
    expect(snap.resolve(anchors, 2.02, targets, .15)).toEqual({ delta: 2, lock: { anchor: 0, target: 2, direction: 1 } });
    expect(snap.resolve(anchors, 2.1, targets, .15).delta).toBe(2);
    expect(snap.resolve(anchors, 2.16, targets, .15)).toEqual({ delta: 2.16, lock: null });
    expect(snap.resolve(anchors, 2.17, targets, .15).lock).toBeNull();
  });
  it('works in both directions and releases immediately when reversing', () => {
    const snap = createDirectionalSnapper(3);
    expect(snap.resolve(anchors, 2.02, targets, .15).lock).toBeNull();
    expect(snap.resolve(anchors, 1.98, targets, .15).lock?.direction).toBe(-1);
    expect(snap.resolve(anchors, 1.99, targets, .15)).toEqual({ delta: 1.99, lock: null });
    // Fine positioning remains free until the next crossing.
    expect(snap.resolve(anchors, 1.995, targets, .15).delta).toBe(1.995);
    expect(snap.resolve(anchors, 2.01, targets, .15).delta).toBe(2);
  });
  it('does not reacquire a latch in the sample that reverses across its edge', () => {
    const snap = createDirectionalSnapper(1);
    snap.resolve(anchors, 2.05, targets, .15);
    expect(snap.resolve(anchors, 1.99, targets, .15)).toEqual({ delta: 1.99, lock: null });
  });
  it('does not snap when toggled at rest or acquire an edge passed while disabled', () => {
    const snap = createDirectionalSnapper(1);
    snap.resolve(anchors, 2.02, targets, .15, false);
    expect(snap.resolve(anchors, 2.02, targets, .15).lock).toBeNull();
    expect(snap.resolve(anchors, 2.05, targets, .15).lock).toBeNull();
    expect(snap.resolve(anchors, 1.99, targets, .15).delta).toBe(2);
    expect(snap.resolve(anchors, 1.98, targets, .15, false).delta).toBe(1.98);
  });
  it('moves a rigid selection from any eligible edge without changing its spacing', () => {
    const snap = createDirectionalSnapper();
    const group = [{ time: 1, edge: 'in' as const }, { time: 3, edge: 'out' as const }, { time: 5, edge: 'out' as const }];
    const result = snap.resolve(group, 1.05, [{ time: 6, edge: 'out' }], .15);
    expect(result).toEqual({ delta: 1, lock: { anchor: 2, target: 6, direction: 1 } });
    expect(group.map(anchor => anchor.time + result.delta)).toEqual([2, 4, 6]);
  });
  it('ignores incompatible edges and never jumps to a distant target crossed by a fast drag', () => {
    const snap = createDirectionalSnapper();
    expect(snap.resolve([{ time: 1, edge: 'in' }], 1.01, [{ time: 2, edge: 'out' }], .15).lock).toBeNull();
    expect(snap.resolve(anchors, 5, targets, .15).lock).toBeNull();
  });
  it('keeps the same latch among dense targets and releases a removed target', () => {
    const snap = createDirectionalSnapper(1);
    const dense = [{ time: 2 }, { time: 2.05 }];
    snap.resolve(anchors, 2.01, dense, .15);
    expect(snap.resolve(anchors, 2.07, dense, .15).delta).toBe(2);
    expect(snap.resolve(anchors, 2.08, [{ time: 2.05 }], .15).lock).toBeNull();
  });
  it('chooses the last crossed target and does not let a weighted target pull backwards', () => {
    const snap = createDirectionalSnapper(1.8);
    expect(snap.resolve(anchors, 2.06, [{ time: 2, weight: 3 }, { time: 2.05 }], .15).delta).toBe(2.05);
  });
  it('does not latch while moving away from an initially aligned edge', () => {
    const snap = createDirectionalSnapper(2);
    expect(snap.resolve(anchors, 2.01, targets, .15).lock).toBeNull();
    snap.reset(1.99);
    expect(snap.resolve(anchors, 2.01, targets, .15).delta).toBe(2);
  });

  it('matches exhaustive snapping through large mixed-edge drags, reversals, toggles and target edits', () => {
    let seed = 74103;
    const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 2 ** 32; };
    const edges = [undefined, 'in', 'out'] as const;
    for (let scenario = 0; scenario < 30; scenario++) {
      const moving = Array.from({ length: 160 }, (): DirectionalSnapAnchor => ({ time: Math.round(random() * 100) / 10, edge: edges[Math.floor(random() * 3)] }));
      const fixed = Array.from({ length: 320 }, (): DirectionalSnapTarget => ({
        time: Math.round(random() * 120) / 10 + (random() < .2 ? random() * 1e-9 : 0),
        edge: edges[Math.floor(random() * 3)], weight: random() < .3 ? 1.5 : undefined,
      }));
      fixed.push({ time: NaN }, { time: Infinity }, { time: -Infinity });
      moving.push({ time: NaN }, { time: Infinity });
      const actual = createDirectionalSnapper(), reference = exhaustiveSnapper();
      let delta = 0;
      for (let sample = 0; sample < 40; sample++) {
        delta += (random() - .5) * 1.5;
        // Targets may change within the same array between pointer samples.
        if (sample % 7 === 0) fixed[Math.floor(random() * 320)]!.time = random() * 12;
        if (sample % 11 === 0) fixed.reverse();
        const requested = sample % 13 === 0 ? NaN : delta;
        const radius = sample % 9 === 0 ? Infinity : sample % 5 === 0 ? 0 : .15;
        const enabled = sample % 6 !== 0;
        expect(actual.resolve(moving, requested, fixed, radius, enabled)).toEqual(reference.resolve(moving, requested, fixed, radius, enabled));
      }
    }
  });

  it.each([0, 1e8, -1e8, 1e15, -1e15, Number.MAX_VALUE / 2])('retains crossing and release boundaries at offset %s', offset => {
    const moving: DirectionalSnapAnchor[] = [{ time: offset }, ...Array.from({ length: 127 }, () => ({ time: NaN }))];
    const fixed: DirectionalSnapTarget[] = [
      { time: offset + 2 + 5e-10, weight: 2 }, { time: offset + 2 },
      { time: offset + 2 - 5e-10, weight: 3 },
      ...Array.from({ length: 200 }, (_, index) => ({ time: offset + index + 100 })),
    ];
    for (const radius of [0, 1e-9, .15, 1000]) {
      const actual = createDirectionalSnapper(1), reference = exhaustiveSnapper(1);
      for (const delta of [2 - 1e-9, 2, 2 + 1e-9, 2.15 + 1e-9, 1.999999999, -2, 1e15]) {
        expect(actual.resolve(moving, delta, fixed, radius)).toEqual(reference.resolve(moving, delta, fixed, radius));
      }
    }
  });

  it('limits target reads for a large selection while choosing the same edge', () => {
    let reads = 0;
    const moving = Array.from({ length: 200 }, (_, index) => ({ time: index / 30 }));
    const fixed = Array.from({ length: 1000 }, (_, index) => ({ get time() { reads++; return index / 30; } }));
    const actual = createDirectionalSnapper(-.1).resolve(moving, .123, fixed, .05);
    expect(reads).toBeLessThan(5000);
    const plain = fixed.map(target => ({ time: target.time }));
    expect(actual).toEqual(exhaustiveSnapper(-.1).resolve(moving, .123, plain, .05));
  });
});
