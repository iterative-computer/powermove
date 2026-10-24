/** Bits per channel. RGBA8 has four 8-bit channels (32 bits per pixel). */
export type ColorDepth = 8 | 16 | 32;

export const COLOR_DEPTH_OPTIONS: Array<{ v: ColorDepth; label: string }> = [
  { v: 8, label: '8-bit' },
  { v: 16, label: '16-bit float' },
  { v: 32, label: '32-bit float' },
];

/** Preserve the renderer's existing precision for older projects. */
export function normalizeColorDepth(value: unknown): ColorDepth {
  return value === 8 || value === 32 ? value : 16;
}

export function supportedColorDepths(gl: { getExtension(name: string): unknown }): ColorDepth[] {
  const half = gl.getExtension('EXT_color_buffer_half_float');
  const float = gl.getExtension('EXT_color_buffer_float');
  // Rendering, filtering, and blending are all needed by the compositor.
  const linear = gl.getExtension('OES_texture_float_linear');
  const blend = gl.getExtension('EXT_float_blend');
  return [8, ...(half || float ? [16 as const] : []), ...(float && linear && blend ? [32 as const] : [])];
}

export function availableColorDepth(requested: ColorDepth, supported: readonly ColorDepth[]): ColorDepth {
  return supported.includes(requested) ? requested : requested === 32 && supported.includes(16) ? 16 : 8;
}

export function colorBufferBytes(width: number, height: number, depth: ColorDepth, withDepth = false): number {
  return Math.max(0, width * height * (depth / 2 + (withDepth ? 2 : 0)));
}
