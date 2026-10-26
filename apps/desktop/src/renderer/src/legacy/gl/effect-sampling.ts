import type { ColorDepth } from '../../core/color-depth';
import { colorBufferBytes } from '../../core/color-depth';

export function sampleLod(radius: number, support: number, maxRadius: number, scale: readonly number[], width: number, height: number): number {
  const extent = Number.isFinite(radius) ? Math.abs(radius) * support * Math.max(...scale.map(Math.abs)) : 0;
  return Math.min(Math.floor(Math.log2(Math.max(1, width, height))), Math.max(0, Math.ceil(Math.log2(Math.max(1, extent / maxRadius)))));
}
export function mipBytes(width: number, height: number, depth: ColorDepth, levels: number): number {
  let bytes = 0;
  for (let level = 1; level <= levels; level++) bytes += colorBufferBytes(Math.max(1, Math.floor(width / 2 ** level)), Math.max(1, Math.floor(height / 2 ** level)), depth);
  return bytes;
}
export function gridSize(columns: number, rows: number, maxSize: number): [number, number] {
  const size = (value: number) => Math.max(1, Math.min(maxSize, Math.round(Number.isFinite(value) ? value : 1)));
  return [size(columns), size(rows)];
}
