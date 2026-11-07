import { describe, expect, it } from 'vitest';
import { colorAt, composeGradient, defaultGradient, gradientGeometry, invert, multiply, nextStopPosition, rawAt, styleName } from './figma-data';

describe('gradient angle and scale', () => {
  it('reads the identity transform as left to right at 100%', () => {
    expect(gradientGeometry([1, 0, 0, 1, 0, 0], 'linear', 90, 60)).toEqual({ angle: 0, scale: 1 });
  });

  it('round-trips angle and scale and keeps the paint centered', () => {
    for (const type of ['linear', 'radial', 'angular', 'diamond']) for (const angle of [-135, -30, 0, 45, 90, 170]) {
      const next = composeGradient([1, 0.2, -0.1, 1, 0, 0], type, 90, 60, angle, 0.75);
      const read = gradientGeometry(next, type, 90, 60);
      expect(read.angle).toBeCloseTo(angle, 1);
      expect(read.scale).toBeCloseTo(0.75, 3);
      const inverse = invert(next);
      expect(inverse[0] * 0.5 + inverse[2] * 0.5 + inverse[4]).toBeCloseTo(
        (() => { const m = invert([1, 0.2, -0.1, 1, 0, 0]); return m[0] * 0.5 + m[2] * 0.5 + m[4]; })(), 5);
    }
  });

  it('spans the box at every angle for linear gradients, as CSS does', () => {
    const vertical = invert(composeGradient([1, 0, 0, 1, 0, 0], 'linear', 90, 60, 90, 1));
    // The axis runs from the top edge to the bottom edge of the layer.
    expect(vertical[0] * 0 + vertical[2] * 0.5 + vertical[4]).toBeCloseTo(0.5, 5);
    expect(vertical[1] * 0 + vertical[3] * 0.5 + vertical[5]).toBeCloseTo(0, 5);
    expect(vertical[1] * 1 + vertical[3] * 0.5 + vertical[5]).toBeCloseTo(1, 5);
  });

  it('inverts affine matrices', () => {
    const m: [number, number, number, number, number, number] = [2, 0.3, -0.4, 1.5, 0.2, -0.1];
    multiply(m, invert(m)).forEach((value, index) => expect(value).toBeCloseTo([1, 0, 0, 1, 0, 0][index]!, 9));
  });
});

describe('stops', () => {
  const stops = [{ position: 0, color: '#FF0000FF' }, { position: 1, color: '#0000FF00' }];
  it('samples the gradient so a new stop does not change the look', () => {
    expect(colorAt(stops, 0.5)).toBe('#80008080');
    expect(colorAt(stops, -1)).toBe('#FF0000FF');
  });
  it('places a new stop in the widest neighbouring gap', () => {
    expect(nextStopPosition(stops, 0)).toBe(0.5);
    expect(nextStopPosition([{ position: 0 }, { position: 0.2 }, { position: 1 }], 1)).toBe(0.6);
  });
  it('starts a gradient from the solid color', () => {
    expect(defaultGradient('#3399FF').stops.map((stop) => stop.color)).toEqual(['#3399FFFF', '#3399FF00']);
  });
});

it('reads nested raw content and names styles', () => {
  const layer = { d: { arc: { start: { v: 1, kf: [] }, end: 2 } } };
  expect(rawAt(layer, 'arc.start')).toEqual({ v: 1, kf: [] });
  expect(rawAt(layer, 'arc.inner')).toBeUndefined();
  expect(styleName(700, true)).toBe('Bold Italic');
  expect(styleName(400, true)).toBe('Italic');
});
