/* Colour math for the picker, after Paper's: sRGB channels as floats that may
   leave 0–1 when a colour came from Display P3, with HSV, HSL, OKLCH, OKLab and
   P3 views of the same value, plus alpha. Documents store sRGB hex (#RRGGBB,
   or #RRGGBBAA below full opacity), so writes clip. */

export type Color = { r: number; g: number; b: number; a: number };
export type Notation = 'hex' | 'rgb' | 'hsl' | 'oklch' | 'display-p3';

export const clamp = (value: number, min = 0, max = 1): number => Math.min(max, Math.max(min, value));
const byte = (value: number): number => Math.round(clamp(value) * 255);
const radians = (degrees: number): number => (degrees * Math.PI) / 180;

export function fromHsv(h: number, s: number, v: number, a = 1): Color {
  const f = (n: number): number => {
    const k = (n + h / 60) % 6;
    return v - v * s * Math.max(Math.min(k, 4 - k, 1), 0);
  };
  return { r: f(5), g: f(3), b: f(1), a };
}

export function toHsv(color: Color): { h: number; s: number; v: number } {
  const { r, g, b } = color;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  let h = 0;
  if (delta) {
    if (max === r) h = ((g - b) / delta) % 6;
    else if (max === g) h = (b - r) / delta + 2;
    else h = (r - g) / delta + 4;
    h = (h * 60 + 360) % 360;
    if (h > 359.9999) h = 0;
  }
  return { h, s: max ? delta / max : 0, v: max };
}

export function fromHsl(h: number, s: number, l: number, a = 1): Color {
  const v = l + s * Math.min(l, 1 - l);
  return fromHsv(h, v ? 2 * (1 - l / v) : 0, v, a);
}

export function toHsl(color: Color): { h: number; s: number; l: number } {
  const { h, s, v } = toHsv(clipped(color));
  const l = v * (1 - s / 2);
  return { h, s: l === 0 || l === 1 ? 0 : (v - l) / Math.min(l, 1 - l), l };
}

const linearize = (c: number): number => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
const delinearize = (c: number): number => c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;

export function toOklch(color: Color): { l: number; c: number; h: number } {
  const r = linearize(color.r), g = linearize(color.g), b = linearize(color.b);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const c = Math.hypot(A, B);
  return { l: L, c: c < 0.00001 ? 0 : c, h: c < 0.00001 ? 0 : (Math.atan2(B, A) * 180 / Math.PI + 360) % 360 };
}

export function toOklab(color: Color): { l: number; a: number; b: number } {
  const { l, c, h } = toOklch(color);
  return { l, a: c * Math.cos(radians(h)), b: c * Math.sin(radians(h)) };
}

export function fromOklab(l: number, a: number, b: number, alpha = 1): Color {
  return fromOklch(l, Math.hypot(a, b), (Math.atan2(b, a) * 180 / Math.PI + 360) % 360, alpha);
}

export function fromOklch(l: number, c: number, h: number, a = 1): Color {
  const A = c * Math.cos(radians(h)), B = c * Math.sin(radians(h));
  const ll = (l + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const mm = (l - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const ss = (l - 0.0894841775 * A - 1.291485548 * B) ** 3;
  return {
    r: delinearize(4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss),
    g: delinearize(-1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss),
    b: delinearize(-0.0041960863 * ll - 0.7034186147 * mm + 1.707614701 * ss),
    a
  };
}

const inGamut = (color: Color): boolean => [color.r, color.g, color.b].every((v) => v >= 0 && v <= 1);

/** Same lightness and hue; chroma drops until the colour fits sRGB. */
export function fromOklchKeepingLightness(l: number, c: number, h: number, a = 1): Color {
  if (inGamut(fromOklch(l, c, h))) return fromOklch(l, c, h, a);
  let low = 0, high = c;
  for (let i = 0; i < 24; i++) {
    const mid = (low + high) / 2;
    if (inGamut(fromOklch(l, mid, h))) low = mid;
    else high = mid;
  }
  return fromOklch(l, low, h, a);
}

/** Same chroma and hue; lightness walks outward until the colour fits sRGB. */
export function fromOklchKeepingChroma(l: number, c: number, h: number, a = 1): Color {
  for (let step = 0; step <= 200; step++) {
    const distance = step * 0.005;
    for (const lightness of step === 0 ? [l] : [l - distance, l + distance]) {
      if (lightness < 0 || lightness > 1) continue;
      const candidate = fromOklch(lightness, c, h, a);
      if (inGamut(candidate)) return candidate;
    }
  }
  return fromOklchKeepingLightness(l, c, h, a);
}

export function toP3(color: Color): Color {
  const r = linearize(color.r), g = linearize(color.g), b = linearize(color.b);
  const x = 0.4123908 * r + 0.35758434 * g + 0.18048079 * b;
  const y = 0.21263901 * r + 0.71516868 * g + 0.07219232 * b;
  const z = 0.01933082 * r + 0.11919478 * g + 0.95053215 * b;
  return {
    r: clamp(delinearize(2.49349691 * x - 0.93138362 * y - 0.40271078 * z)),
    g: clamp(delinearize(-0.82948897 * x + 1.76266406 * y + 0.02362469 * z)),
    b: clamp(delinearize(0.03584583 * x - 0.07617239 * y + 0.95688452 * z)),
    a: color.a
  };
}

export function fromP3(color: Color): Color {
  const r = linearize(color.r), g = linearize(color.g), b = linearize(color.b);
  const x = 0.48657095 * r + 0.26566769 * g + 0.19821729 * b;
  const y = 0.22897456 * r + 0.69173852 * g + 0.07928691 * b;
  const z = 0.04511338 * g + 1.04394437 * b;
  return {
    r: delinearize(3.24096994 * x - 1.53738318 * y - 0.49861076 * z),
    g: delinearize(-0.96924364 * x + 1.8759675 * y + 0.04155506 * z),
    b: delinearize(0.05563008 * x - 0.20397696 * y + 1.05697151 * z),
    a: color.a
  };
}

export const clipped = (color: Color): Color => ({ r: clamp(color.r), g: clamp(color.g), b: clamp(color.b), a: color.a });

export const outsideSrgb = (color: Color): boolean => [color.r, color.g, color.b].some((c) => c < -0.00001 || c > 1.00001);

const pair = (c: number): string => byte(c).toString(16).padStart(2, '0');

/** Six hex digits for the colour alone, the way the value field shows it. */
export function hex(color: Color): string {
  return `#${[color.r, color.g, color.b].map(pair).join('').toUpperCase()}`;
}

/** The value a document stores: sRGB hex, clipped; alpha only when it is not opaque. */
export function storedHex(color: Color): string {
  return byte(color.a) === 255 ? hex(color) : `${hex(color)}${pair(color.a).toUpperCase()}`;
}

export function cssColor(color: Color): string {
  if (outsideSrgb(color)) {
    const p3 = toP3(color);
    return `color(display-p3 ${p3.r} ${p3.g} ${p3.b} / ${clamp(color.a)})`;
  }
  return `rgba(${byte(color.r)}, ${byte(color.g)}, ${byte(color.b)}, ${clamp(color.a)})`;
}

/* Alpha is stored as one byte, so 50% comes back as 128 / 255 = 50.2%. Show the
   whole percent whenever it stores the same byte. */
export function opacityText(alpha: number): string {
  const whole = Math.round(clamp(alpha) * 100);
  if (byte(whole / 100) === byte(alpha)) return String(whole);
  const number = Math.round(clamp(alpha) * 1000) / 10;
  return Number.isInteger(number) ? String(number) : number.toFixed(1);
}

export function formatColor(color: Color, notation: Notation): string {
  if (notation === 'hex') return hex(color);
  if (notation === 'display-p3') {
    const p3 = toP3(color);
    return [p3.r, p3.g, p3.b].map((n) => String(Number(n.toFixed(3)))).join(' ');
  }
  if (notation === 'rgb') return `${byte(color.r)} ${byte(color.g)} ${byte(color.b)}`;
  if (notation === 'hsl') {
    const { h, s, l } = toHsl(color);
    return `${Math.round(h)}° ${Math.round(s * 100)}% ${Math.round(l * 100)}%`;
  }
  const { l, c, h } = toOklch(color);
  return `${Math.round(l * 100)}% ${c.toFixed(3)} ${Math.round(h)}°`;
}

function components(input: string): string[] {
  return input.replace(/,/g, ' ').replace(/\s*\/\s*/, ' / ').trim().split(/\s+/);
}
const fraction = (input: string): number => input.endsWith('%') ? Number.parseFloat(input) / 100 : Number.parseFloat(input);
const alphaOf = (input?: string): number => input === undefined || input === '' ? 1 : clamp(fraction(input));

/** Any CSS-ish colour a person might paste, with or without alpha. */
export function parseColor(raw: string): Color | null {
  const input = raw.trim().toLowerCase();
  if (input === 'transparent') return { r: 0, g: 0, b: 0, a: 0 };
  const hexMatch = input.match(/^#?([\da-f]{3,4}|[\da-f]{6}|[\da-f]{8})$/i);
  if (hexMatch) {
    let value = hexMatch[1]!;
    if (value.length <= 4) value = [...value].map((c) => c + c).join('');
    return {
      r: parseInt(value.slice(0, 2), 16) / 255, g: parseInt(value.slice(2, 4), 16) / 255,
      b: parseInt(value.slice(4, 6), 16) / 255, a: value.length === 8 ? parseInt(value.slice(6, 8), 16) / 255 : 1
    };
  }
  const fn = input.match(/^([a-z-]+)\((.*)\)$/);
  if (!fn) return null;
  const name = fn[1]!;
  const values = components(fn[2]!).filter((v) => v !== '/');
  if (values.slice(name === 'color' ? 1 : 0).some((v) => !Number.isFinite(Number.parseFloat(v)))) return null;
  const [first = '', second = '', third = '', fourth = '', fifth] = values;
  if ((name === 'rgb' || name === 'rgba') && values.length >= 3) {
    const channel = (v: string): number => clamp(v.endsWith('%') ? fraction(v) : Number(v) / 255);
    return { r: channel(first), g: channel(second), b: channel(third), a: alphaOf(fourth) };
  }
  if ((name === 'hsl' || name === 'hsla') && values.length >= 3)
    return fromHsl(Number.parseFloat(first), clamp(fraction(second)), clamp(fraction(third)), alphaOf(fourth));
  if ((name === 'hsb' || name === 'hsv') && values.length >= 3)
    return fromHsv(Number.parseFloat(first), clamp(fraction(second)), clamp(fraction(third)), alphaOf(fourth));
  if (name === 'oklch' && values.length >= 3)
    return fromOklch(fraction(first), Number(second), Number(third), alphaOf(fourth));
  if (name === 'color' && first === 'display-p3' && values.length >= 4)
    return fromP3({ r: Number(second), g: Number(third), b: Number(fourth), a: alphaOf(fifth) });
  return null;
}
