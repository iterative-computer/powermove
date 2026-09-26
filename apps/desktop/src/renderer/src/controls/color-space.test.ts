import { describe, expect, it } from 'vitest';
import { formatColor, fromOklchKeepingLightness, fromP3, hex, opacityText, outsideSrgb, parseColor, storedHex, toOklch } from './color-space';

describe('color space', () => {
  it('parses the notations a person pastes, with their alpha', () => {
    expect(storedHex(parseColor('#ff8800')!)).toBe('#FF8800');
    expect(storedHex(parseColor('rgb(255, 136, 0)')!)).toBe('#FF8800');
    expect(storedHex(parseColor('hsl(32 100% 50%)')!)).toBe('#FF8800');
    expect(storedHex(parseColor('hsb(240, 100%, 100%)')!)).toBe('#0000FF');
    expect(storedHex(parseColor('#ff880080')!)).toBe('#FF880080');
    expect(storedHex(parseColor('rgb(255 136 0 / 25%)')!)).toBe('#FF880040');
    expect(storedHex(parseColor('transparent')!)).toBe('#00000000');
    expect(parseColor('rgb(255, 0, 0')).toBeNull();
  });

  it('stores six digits while opaque so existing documents keep their values', () => {
    expect(storedHex({ r: 1, g: 0, b: 0, a: 1 })).toBe('#FF0000');
    expect(storedHex({ r: 1, g: 0, b: 0, a: 0.999 })).toBe('#FF0000');
    expect(storedHex({ r: 1, g: 0, b: 0, a: 0.5 })).toBe('#FF000080');
    expect(hex({ r: 1, g: 0, b: 0, a: 0.5 })).toBe('#FF0000');
    expect(opacityText(parseColor('#FF000080')!.a)).toBe('50');
    expect(opacityText(0.333)).toBe('33.3');
  });

  it('keeps Display P3 colours outside sRGB and clips only when writing hex', () => {
    const red = parseColor('color(display-p3 1 0 0)')!;
    expect(outsideSrgb(red)).toBe(true);
    expect(hex(red)).toBe('#FF0000');
    expect(formatColor(red, 'display-p3')).toBe('1 0 0');
    expect(outsideSrgb(fromP3({ r: 0.5, g: 0.5, b: 0.5, a: 1 }))).toBe(false);
  });

  it('formats each notation the way the value field shows it', () => {
    const orange = parseColor('#FF6B1A')!;
    expect(formatColor(orange, 'rgb')).toBe('255 107 26');
    expect(formatColor(orange, 'hsl')).toBe('21° 100% 55%');
    expect(formatColor(orange, 'oklch')).toMatch(/^70% 0\.19\d \d+°$/);
  });

  it('holds lightness while reducing chroma into gamut', () => {
    const fitted = fromOklchKeepingLightness(0.7, 0.4, 140);
    expect(outsideSrgb(fitted)).toBe(false);
    expect(toOklch(fitted).l).toBeCloseTo(0.7, 3);
  });
});
