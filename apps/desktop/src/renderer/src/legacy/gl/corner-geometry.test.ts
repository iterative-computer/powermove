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

type Point = { x: number; y: number };
const values = (path: string) => path.match(/-?\d*\.?\d+(?:e-?\d+)?/g)!.map(Number);

/** Sample the actual serialized outline, including its circular SVG arcs. */
function samples(path: string): Point[] {
  const tokens = path.match(/[MLlcaZ]|-?\d*\.?\d+(?:e-?\d+)?/g)!;
  let pen: Point = { x: 0, y: 0 }, start = pen;
  const points: Point[] = [];
  for (let i = 0; i < tokens.length;) {
    const cmd = tokens[i++]!;
    const take = (n: number) => tokens.slice(i, i += n).map(Number);
    if (cmd === 'M') { const [x, y] = take(2) as [number, number]; pen = start = { x, y }; continue; }
    const from = pen;
    if (cmd === 'c') {
      const [ax, ay, bx, by, x, y] = take(6) as [number, number, number, number, number, number];
      for (let j = 0; j <= 20; j++) {
        const t = j / 20, u = 1 - t;
        points.push({ x: from.x + 3*u*u*t*ax + 3*u*t*t*bx + t*t*t*x,
          y: from.y + 3*u*u*t*ay + 3*u*t*t*by + t*t*t*y });
      }
      pen = { x: from.x + x, y: from.y + y };
    } else if (cmd === 'a') {
      const [r, , , , , x, y] = take(7) as [number, number, number, number, number, number, number];
      const chord = Math.hypot(x, y);
      if (chord === 0) { for (let j = 0; j <= 20; j++) points.push(from); continue; }
      const offset = Math.sqrt(Math.max(0, r*r - chord*chord/4));
      const cx = from.x + x/2 - y/chord*offset, cy = from.y + y/2 + x/chord*offset;
      const angle = Math.atan2(from.y - cy, from.x - cx);
      const sweep = 2 * Math.asin(Math.min(1, chord / (2*r)));
      for (let j = 0; j <= 20; j++) points.push({ x: cx + r*Math.cos(angle + sweep*j/20), y: cy + r*Math.sin(angle + sweep*j/20) });
      pen = { x: from.x + x, y: from.y + y };
    } else {
      const v = cmd === 'Z' ? [start.x, start.y] : take(2);
      pen = cmd === 'l' ? { x: from.x + v[0]!, y: from.y + v[1]! } : { x: v[0]!, y: v[1]! };
      for (let j = 0; j <= 20; j++) points.push({ x: from.x + (pen.x - from.x)*j/20, y: from.y + (pen.y - from.y)*j/20 });
    }
  }
  return points;
}

/** Rotate each corner into the top-right corner's local coordinate system. */
function corners(path: string) {
  return [...path.matchAll(/c ([^a]+) a ([^c]+) c ([^LZ]+)/g)].map(match => {
    const first = values(match[1]!), arcValues = values(match[2]!);
    // Sharp corners have no cubic, so derive orientation rather than counting.
    const i = first[0] ? (first[0] > 0 ? 0 : 2) : first[1] ? (first[1] > 0 ? 1 : 3)
      : arcValues[5]! >= 0 ? (arcValues[6]! >= 0 ? 0 : 3) : (arcValues[6]! >= 0 ? 1 : 2);
    const rotate = (v: number[]) => {
      const cos = [1, 0, -1, 0][i]!, sin = [0, 1, 0, -1][i]!;
      return Array.from({ length: v.length / 2 }, (_, j) => [v[j*2]!*cos + v[j*2+1]!*sin, -v[j*2]!*sin + v[j*2+1]!*cos]).flat();
    };
    const entry = rotate(values(match[1]!)), exit = rotate(values(match[3]!)), arc = values(match[2]!);
    return { index: i, a: entry[0]!, b: entry[2]! - entry[0]!, c: entry[4]! - entry[2]!, d: entry[5]!, r: arc[0]!, arc: Math.abs(arc[5]!), entry, exit };
  });
}

function maxDeviation(a: Point[], b: Point[]): number {
  expect(a).toHaveLength(b.length);
  return Math.max(...a.map((p, i) => Math.hypot(p.x - b[i]!.x, p.y - b[i]!.y)));
}

describe('round pills and continuous smoothing', () => {
  it.each([[520, 100], [100, 520], [100, 100]])('keeps exact circular ends at smoothing 0/60/100 for %j', (w, h) => {
    const radius = Math.min(w, h) / 2;
    const plain = roundedRectPath({ w, h, radius, smoothing: 0 });
    for (const smoothing of [0, 60, 100]) {
      const path = roundedRectPath({ w, h, radius, smoothing });
      expect(path).toBe(plain);
      const points = samples(path);
      // Every sampled point lies on the analytic capsule (circle for a square).
      for (const p of points) {
        const cx = Math.min(w - radius, Math.max(radius, p.x));
        const cy = Math.min(h - radius, Math.max(radius, p.y));
        expect(Math.abs(Math.hypot(p.x - cx, p.y - cy) - radius)).toBeLessThan(.0003);
      }
    }
  });

  it.each([0, 1, 60, 100])('keeps the radius and full smoothing when there is room at %s%%', smoothing => {
    const path = roundedRectPath({ w: 300, h: 100, radius: 20, smoothing });
    const corner = corners(path)[0]!;
    expect(corner.r).toBe(20);
    expect(values(path)[0]).toBeCloseTo(300 - 20*(1 + smoothing/100), 4);
    expect(corner.arc).toBeCloseTo(20 * Math.SQRT2 * Math.sin(Math.PI/4*(1 - smoothing/100)), 4);
  });

  it('retains some smoothing just below the maximum', () => {
    expect(roundedRectPath({ w: 300, h: 100, radius: 49, smoothing: 60 }))
      .not.toBe(roundedRectPath({ w: 300, h: 100, radius: 49, smoothing: 0 }));
    expect(corners(roundedRectPath({ w: 300, h: 100, radius: 49, smoothing: 60 }))[0]!.r).toBe(49);
  });

  it.each([0, 1, 25, 60, 100])('has no point jump while dragging radius or size at %s%%', smoothing => {
    const shapes = [
      (n: number) => ({ w: 300, h: 100, radius: n, smoothing }),
      (n: number) => ({ w: 100, h: 300, radius: n, smoothing }),
      (n: number) => ({ w: 300, h: n*2, radius: 40, smoothing }),
      (n: number) => ({ w: 300, h: 100, radius: 0, independentCorners: true, radiusTL: n, radiusTR: 15, radiusBR: 50, radiusBL: 40, smoothing }),
    ];
    for (const shape of shapes) {
      // Includes the full-smoothing budget boundary and maximum-radius clamp.
      let previous = samples(roundedRectPath(shape(1)));
      for (let step = 11; step <= 700; step++) {
        const next = samples(roundedRectPath(shape(step/10)));
        expect(maxDeviation(previous, next)).toBeLessThan(.5);
        previous = next;
      }
    }
  });

  it('changes continuously when smoothing is animated below and at the limit', () => {
    for (const radius of [20, 40, 49, 50]) {
      let previous = samples(roundedRectPath({ w: 300, h: 100, radius, smoothing: 0 }));
      for (let smoothing = 1; smoothing <= 100; smoothing++) {
        const next = samples(roundedRectPath({ w: 300, h: 100, radius, smoothing }));
        expect(maxDeviation(previous, next)).toBeLessThan(.6);
        previous = next;
      }
    }
  });

  it('keeps all corners finite, inside the box and monotone with nonnegative segment lengths', () => {
    for (const [w, h] of [[300, 100], [100, 300], [100, 100], [0, 100]]) {
      for (const smoothing of [0, .01, 1, 25, 60, 99.99, 100]) {
        for (const radius of [0, .01, 1, 20, 31.25, 40, 49.99, 50, 100]) {
          for (const independentCorners of [false, true]) {
            const path = roundedRectPath({ w, h, radius, smoothing, independentCorners, radiusTL: radius, radiusTR: 0, radiusBR: 50, radiusBL: 7 });
            expect(values(path).every(Number.isFinite)).toBe(true);
            const reach = [0, 0, 0, 0]; // TR, BR, BL, TL
            for (const k of corners(path)) {
              for (const n of [k.a, k.b, k.c, k.d, k.r, k.arc]) expect(n).toBeGreaterThanOrEqual(-.00011);
              reach[k.index] = k.a + k.b + k.c + k.d + k.arc;
            }
            // Neighbouring corners never reverse the straight edge between them.
            for (let i = 0; i < 4; i++) expect(reach[i]! + reach[(i+1)%4]!).toBeLessThanOrEqual((i%2 ? w! : h!) + .0004);
            for (const p of samples(path)) {
              expect(p.x).toBeGreaterThanOrEqual(-.001); expect(p.x).toBeLessThanOrEqual(w! + .001);
              expect(p.y).toBeGreaterThanOrEqual(-.001); expect(p.y).toBeLessThanOrEqual(h! + .001);
            }
          }
        }
      }
    }
  });

  it('matches tangents and curvature at both cubic/arc joins, including budget-reduced corners', () => {
    // Large coordinates keep serialized 0.0001-unit rounding below curvature tolerance.
    for (const [w, h] of [[600000, 200000], [200000, 600000], [200000, 200000]]) {
      for (const radius of [10000, 50000, 62500, 80000, 99900, 100000]) {
        for (const smoothing of [1, 10, 60, 90, 100]) {
          for (const independentCorners of [false, true]) {
            const path = roundedRectPath({ w, h, radius, smoothing, independentCorners, radiusTL: radius, radiusTR: 25000, radiusBR: 100000, radiusBL: 60000 });
            for (const k of corners(path)) {
              if (k.d === 0) { // Exact circle: collapsed cubics, tangent at the edge.
                expect(k.arc).toBe(k.r);
                expect([k.a, k.b, k.c]).toEqual([0, 0, 0]); continue;
              }
              // Entry's first two controls and exit's last two are collinear with the edges.
              expect(k.entry[1]).toBe(0); expect(k.entry[3]).toBe(0);
              expect(k.exit[0]).toBe(k.exit[2]); expect(k.exit[2]).toBe(k.exit[4]);
              expect(k.a).toBeGreaterThan(0);
              const curvature = 2*k.b*k.d/(3*Math.hypot(k.c, k.d)**3);
              expect(Math.abs(curvature*k.r - 1)).toBeLessThan(.003);
              const exitB = k.exit[3]! - k.exit[1]!;
              const exitCurvature = 2*exitB*k.exit[0]!/(3*Math.hypot(k.exit[0]!, k.exit[1]!)**3);
              expect(Math.abs(exitCurvature*k.r - 1)).toBeLessThan(.003);
              const beta = Math.atan2(k.d, k.c);
              // A circular arc turns through 90° minus the two cubic tangent angles.
              expect(Math.abs(2*Math.asin(Math.min(1, k.arc/(Math.SQRT2*k.r))) + 2*beta - Math.PI/2)).toBeLessThan(.0001);
              expect(Math.abs(Math.atan2(k.exit[1]!, k.exit[0]!) + beta - Math.PI/2)).toBeLessThan(.0001);
            }
          }
        }
      }
    }
  });
});
