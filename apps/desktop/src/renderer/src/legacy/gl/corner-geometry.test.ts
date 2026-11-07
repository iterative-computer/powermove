import { describe, expect, it } from 'vitest';
import { cornerInset, cornerRadii, cornerSmoothing, isUniformCircular, roundedRectPath } from './corner-geometry';

/** Walk the path's relative/absolute commands and return the final pen position. */
function endpoint(path: string): { x: number; y: number } {
  let x = 0, y = 0;
  const tokens = path.match(/[MLlcaZ]|-?\d*\.?\d+(?:e-?\d+)?/g)!;
  for (let i = 0; i < tokens.length;) {
    const cmd = tokens[i++]!;
    const take = (n: number) => tokens.slice(i, i += n).map(Number);
    if (cmd === 'M' || cmd === 'L') [x, y] = take(2) as [number, number];
    else if (cmd === 'l') { const [dx, dy] = take(2) as [number, number]; x += dx; y += dy; }
    else if (cmd === 'c') { const v = take(6); x += v[4]!; y += v[5]!; }
    else if (cmd === 'a') { const v = take(7); x += v[5]!; y += v[6]!; }
  }
  return { x, y };
}

describe('corner geometry', () => {
  it('uses the uniform radius unless corners are independent', () => {
    expect(cornerRadii({ radius: 12, radiusTL: 3 })).toEqual([12, 12, 12, 12]);
    expect(cornerRadii({ radius: 12, independentCorners: true, radiusTL: 1, radiusBR: 4 })).toEqual([1, 12, 4, 12]);
  });

  it('reads smoothing as a clamped percentage', () => {
    expect(cornerSmoothing({ smoothing: 60 })).toBeCloseTo(0.6);
    expect(cornerSmoothing({ smoothing: 250 })).toBe(1);
    expect(cornerSmoothing({})).toBe(0);
  });

  it('keeps the plain circular rectangle on the legacy path only when nothing differs', () => {
    expect(isUniformCircular({ radius: 8 })).toBe(true);
    expect(isUniformCircular({ radius: 8, smoothing: 1 })).toBe(false);
    expect(isUniformCircular({ radius: 8, independentCorners: true, radiusTL: 2 })).toBe(false);
  });

  it('traces a closed outline back onto the top edge', () => {
    for (const d of [
      { w: 200, h: 100, radius: 24, smoothing: 0 },
      { w: 200, h: 100, radius: 24, smoothing: 60 },
      { w: 200, h: 100, radius: 500, smoothing: 100 },
      { w: 200, h: 100, radius: 0, independentCorners: true, radiusTL: 40, radiusBR: 10, smoothing: 50 },
    ]) {
      const path = roundedRectPath(d);
      const start = path.match(/^M (-?[\d.]+) (-?[\d.]+)/)!;
      const end = endpoint(path.replace(/ Z$/, ''));
      // The outline starts and ends on the top edge; Z closes it with a straight segment.
      expect(end.y).toBeCloseTo(Number(start[2]), 3);
      expect(end.x).toBeLessThanOrEqual(Number(start[1]) + 1e-6);
      expect(path).not.toContain('NaN');
    }
  });

  it('reaches further along the edge as smoothing grows', () => {
    const plain = roundedRectPath({ w: 200, h: 100, radius: 20, smoothing: 0 });
    const smooth = roundedRectPath({ w: 200, h: 100, radius: 20, smoothing: 100 });
    expect(plain.startsWith('M 180 0')).toBe(true);
    expect(smooth.startsWith('M 160 0')).toBe(true);
  });

  it('insets by the widest corner including smoothing', () => {
    expect(cornerInset({ w: 200, h: 100, radius: 10 })).toBe(10);
    expect(cornerInset({ w: 200, h: 100, radius: 10, smoothing: 50 })).toBe(15);
    expect(cornerInset({ w: 200, h: 100, radius: 900, smoothing: 100 })).toBe(50);
  });
});

describe('continuous capsule smoothing', () => {
  const values = (path: string) => path.match(/-?\d*\.?\d+(?:e-?\d+)?/g)!.map(Number);
  it.each([[520, 100], [100, 520], [100, 100]])('retains distinct smoothing at full radius for %j', (w, h) => {
    const paths = [0, 50, 100].map(smoothing => roundedRectPath({ w, h, radius: Math.min(w, h) / 2, smoothing }));
    expect(new Set(paths).size).toBe(3);
    for (const path of paths) expect(values(path).every(Number.isFinite)).toBe(true);
  });

  it.each([0, 25, 50, 75, 100])('has no jump across the maximum radius at smoothing %s', smoothing => {
    const at = (radius: number) => roundedRectPath({ w: 300, h: 100, radius, smoothing });
    const below = values(at(49.999)), limit = values(at(50)), above = values(at(50.001));
    expect(below).toHaveLength(limit.length);
    below.forEach((value, i) => expect(Math.abs(value - limit[i]!)).toBeLessThan(.005));
    expect(above).toEqual(limit);
  });

  it('changes continuously when smoothing is animated at maximum radius', () => {
    let previous = values(roundedRectPath({ w: 300, h: 100, radius: 50, smoothing: 0 }));
    for (let smoothing = 1; smoothing <= 100; smoothing++) {
      const next = values(roundedRectPath({ w: 300, h: 100, radius: 50, smoothing }));
      next.forEach((value, i) => expect(Math.abs(value - previous[i]!)).toBeLessThan(1.6));
      previous = next;
    }
  });

  it('meets straight edges tangentially with zero curvature when smoothing is nonzero', () => {
    for (const smoothing of [1, 50, 100]) {
      const path = roundedRectPath({ w: 300, h: 100, radius: 50, smoothing });
      const first = path.match(/ c ([^a]+) a/)!;
      const controls = values(first[1]!);
      // Collinear first controls mean a horizontal tangent and zero edge curvature.
      expect(controls[1]).toBe(0); expect(controls[3]).toBe(0);
      expect(controls[0]).toBeGreaterThan(0);
      expect(controls[4]).toBeGreaterThan(controls[2]!);
      expect(controls[5]).toBeGreaterThanOrEqual(0);
    }
  });
});


it.each([10, 50, 90])('matches curvature at cubic/arc joins at smoothing %s', smoothing => {
  const path = roundedRectPath({ w: 6000, h: 2000, radius: 1000, smoothing });
  const segment = path.match(/ c ([^a]+) a ([^c]+)/)!;
  const controls = segment[1]!.trim().split(/\s+/).map(Number);
  const radius = Number(segment[2]!.trim().split(/\s+/)[0]);
  const b = controls[2]! - controls[0]!, c = controls[4]! - controls[2]!, d = controls[5]!;
  const curvature = 2 * b * d / (3 * Math.hypot(c, d) ** 3);
  expect(curvature).toBeCloseTo(1 / radius, 6);
});
