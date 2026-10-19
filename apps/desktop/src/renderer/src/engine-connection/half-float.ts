// Binary16 conversion shared by the fallback readback paths.
const scratchF32 = new Float32Array(1);
const scratchU32 = new Uint32Array(scratchF32.buffer);

/** f32 to binary16 bits, round to nearest even (values from RGBA16F storage convert exactly). */
export function toHalf(value: number): number {
  scratchF32[0] = value;
  return halfBits(scratchU32[0]!);
}

function halfBits(x: number): number {
  const sign = (x >>> 16) & 0x8000;
  const exp = (x >>> 23) & 0xff;
  let mant = x & 0x7fffff;
  if (exp === 0xff) return sign | 0x7c00 | (mant ? 0x200 : 0);
  const e = exp - 127 + 15;
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

/** Converts a view without allocating per component; respects its byte offset. */
export function toHalfArray(src: Float32Array): Uint16Array {
  const bits = new Uint32Array(src.buffer, src.byteOffset, src.length);
  const dst = new Uint16Array(src.length);
  for (let i = 0; i < bits.length; i++) dst[i] = halfBits(bits[i]!);
  return dst;
}
