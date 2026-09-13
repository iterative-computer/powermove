import type { PowermoveAPI } from '../kernel/api';

type ControlUtilAPI = Pick<PowermoveAPI, 'model' | 'util'>;

export interface FillStop {
  id: string;
  color: string;
  position: number;
}

export interface FillValue {
  type: 'solid' | 'linear' | 'radial' | 'none';
  angle: number;
  stops: FillStop[];
}

export const clamp = (value: number, min: number, max: number): number => Math.max(min, Math.min(max, value));

export const round = (api: Pick<PowermoveAPI, 'util'>, value: number, precision: number): number =>
  api.util.round(value, precision);

export const normalizeHex = (value: unknown): string | null => {
  const raw = String(value ?? '').trim().replace(/^#/, '');
  if (/^[0-9a-f]{3}$/i.test(raw)) return `#${raw.split('').map((char) => char + char).join('').toUpperCase()}`;
  return /^[0-9a-f]{6}$/i.test(raw) ? `#${raw.toUpperCase()}` : null;
};

const colorChannel = (value: unknown, max = 255): number => Math.round(clamp(Number(value) || 0, 0, max));

export const rgbToHex = ({ r, g, b }: { r: unknown; g: unknown; b: unknown }): string =>
  `#${[r, g, b].map((value) => colorChannel(value).toString(16).padStart(2, '0')).join('').toUpperCase()}`;

export const hexToRgb = (value: unknown): { r: number; g: number; b: number } | null => {
  const hex = normalizeHex(value);
  return hex ? { r: parseInt(hex.slice(1, 3), 16), g: parseInt(hex.slice(3, 5), 16), b: parseInt(hex.slice(5, 7), 16) } : null;
};

export const rgbToHsv = ({ r, g, b }: { r: number; g: number; b: number }): { h: number; s: number; v: number } => {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), difference = max - min;
  let h = 0;
  if (difference) h = max === r ? 60 * (((g - b) / difference) % 6) : max === g ? 60 * ((b - r) / difference + 2) : 60 * ((r - g) / difference + 4);
  if (h < 0) h += 360;
  return { h: Math.round(h), s: Math.round(max ? difference / max * 100 : 0), v: Math.round(max * 100) };
};

export const hsvToRgb = ({ h, s, v }: { h: unknown; s: unknown; v: unknown }): { r: number; g: number; b: number } => {
  const hue = ((Number(h) || 0) % 360 + 360) % 360;
  const saturation = clamp(Number(s) || 0, 0, 100) / 100;
  const brightness = clamp(Number(v) || 0, 0, 100) / 100;
  const c = brightness * saturation, x = c * (1 - Math.abs((hue / 60) % 2 - 1)), m = brightness - c;
  const parts = hue < 60 ? [c, x, 0] : hue < 120 ? [x, c, 0] : hue < 180 ? [0, c, x] : hue < 240 ? [0, x, c] : hue < 300 ? [x, 0, c] : [c, 0, x];
  return { r: Math.round((parts[0]! + m) * 255), g: Math.round((parts[1]! + m) * 255), b: Math.round((parts[2]! + m) * 255) };
};

export const fillCss = (fill: FillValue): string => {
  if (fill.type === 'none') return 'transparent';
  if (fill.type === 'solid') return fill.stops[0]?.color ?? '#000000';
  const stops = fill.stops.map((stop) => `${stop.color} ${stop.position}%`).join(',');
  return fill.type === 'radial' ? `radial-gradient(circle,${stops})` : `linear-gradient(${fill.angle}deg,${stops})`;
};

export function normalizeFill(api: ControlUtilAPI, value: unknown, fallback = '#000000'): FillValue {
  return api.model.normalizeFill(value, fallback) as FillValue;
}
