/*
 * Deterministic test inputs, premultiplied RGBA quantized to f16 so the WebGL
 * and wgpu uploads are bit-identical. Rows are stored bottom-up (GL row
 * order), which is what texImage2D expects without UNPACK_FLIP_Y.
 *
 * The images are 256x144 (non-square, non-power-of-two, so orientation and
 * aspect mistakes show) and contain hard edges, antialiased edges, partial
 * alpha, fully transparent regions, thin lines, and a 1-px checker that
 * exercises bilinear filtering.
 */
export const WIDTH = 256;
export const HEIGHT = 144;

const f32 = new Float32Array(1);
const u32 = new Uint32Array(f32.buffer);
/** f32 to f16 bits, round to nearest even (handles subnormals, inf, nan). */
export function toHalf(value: number): number {
  f32[0] = value;
  const x = u32[0]!;
  const sign = (x >>> 16) & 0x8000;
  const exp = (x >>> 23) & 0xff;
  let mant = x & 0x7fffff;
  if (exp === 0xff) return sign | 0x7c00 | (mant ? 0x200 : 0);
  let e = exp - 127 + 15;
  if (e >= 0x1f) return sign | 0x7c00;
  if (e <= 0) {
    if (e < -10) return sign;
    mant |= 0x800000;
    const shift = 14 - e;
    let half = mant >>> shift;
    const rem = mant & ((1 << shift) - 1);
    const mid = 1 << (shift - 1);
    if (rem > mid || (rem === mid && (half & 1))) half++;
    return sign | half;
  }
  let half = (e << 10) | (mant >>> 13);
  const rem = mant & 0x1fff;
  if (rem > 0x1000 || (rem === 0x1000 && (half & 1))) half++;
  return sign | half;
}

type Pixel = [number, number, number, number]; // straight RGBA in 0..1
const smooth = (e0: number, e1: number, x: number) => {
  const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)));
  return t * t * (3 - 2 * t);
};
const over = (top: Pixel, bottom: Pixel): Pixel => {
  const a = top[3] + bottom[3] * (1 - top[3]);
  if (a <= 0) return [0, 0, 0, 0];
  const mix = (i: number) => (top[i]! * top[3] + bottom[i]! * bottom[3] * (1 - top[3])) / a;
  return [mix(0), mix(1), mix(2), a];
};

/** x, y in pixels from the top-left (visual coordinates). */
function layer(x: number, y: number): Pixel {
  let p: Pixel = [0, 0, 0, 0];
  // Opaque rounded rectangle with a horizontal gradient (hard-ish AA edge).
  const rx = Math.max(Math.abs(x - 92) - 54, 0), ry = Math.max(Math.abs(y - 60) - 30, 0);
  const box = Math.hypot(rx, ry) - 10;
  p = over([0.95 - x / 400, 0.35 + y / 300, 0.2 + x / 500, 1 - smooth(-0.5, 0.5, box)], p);
  // Soft translucent circle.
  const d = Math.hypot(x - 180, y - 84) - 38;
  p = over([0.2, 0.55, 1.0, 0.6 * (1 - smooth(-6, 6, d))], p);
  // Thin text-like bars.
  if (y >= 112 && y < 116 && x > 24 && x < 140 && Math.floor(x / 6) % 3 !== 0) p = over([1, 1, 1, 1], p);
  if (x >= 210 && x < 211 && y > 10 && y < 130) p = over([1, 0.9, 0.2, 1], p);
  // 1-px checker patch (sampling stress).
  if (x >= 220 && x < 248 && y >= 100 && y < 136) p = over((x + y) % 2 ? [0.9, 0.1, 0.4, 1] : [0.05, 0.05, 0.05, 1], p);
  return p;
}
function backdrop(x: number, y: number): Pixel {
  const stripes = Math.floor((x + y * 0.5) / 12) % 2 ? 0.12 : 0;
  return [0.1 + 0.6 * (x / WIDTH) + stripes, 0.25 + 0.5 * (y / HEIGHT), 0.55 - 0.3 * (x / WIDTH) + stripes, 1];
}
function from(x: number, y: number): Pixel {
  return over(layer(x, y), [0.08, 0.07, 0.1, 1]);
}
function to(x: number, y: number): Pixel {
  const r = Math.hypot(x - WIDTH * 0.6, y - HEIGHT * 0.4);
  const ring = 0.5 + 0.5 * Math.cos(r / 7);
  return [0.9 * ring, 0.4 + 0.3 * (x / WIDTH), 0.25 + 0.6 * (1 - ring), 1];
}

export interface ImageFile { name: string; width: number; height: number; file: string; bytes: Uint8Array }

export function makeImages(): ImageFile[] {
  return ([['layer', layer], ['backdrop', backdrop], ['from', from], ['to', to]] as const).map(([name, fn]) => {
    const halfs = new Uint16Array(WIDTH * HEIGHT * 4);
    for (let row = 0; row < HEIGHT; row++) {
      const y = HEIGHT - 1 - row; // GL row order: row 0 is the bottom
      for (let x = 0; x < WIDTH; x++) {
        const [r, g, b, a] = fn(x + 0.5, y + 0.5);
        const at = (row * WIDTH + x) * 4;
        // premultiplied, as layer buffers are in the compositor
        halfs[at] = toHalf(Math.min(1, Math.max(0, r)) * a);
        halfs[at + 1] = toHalf(Math.min(1, Math.max(0, g)) * a);
        halfs[at + 2] = toHalf(Math.min(1, Math.max(0, b)) * a);
        halfs[at + 3] = toHalf(a);
      }
    }
    return { name, width: WIDTH, height: HEIGHT, file: `images/${name}.f16`, bytes: new Uint8Array(halfs.buffer) };
  });
}
