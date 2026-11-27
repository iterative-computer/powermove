/* Images in Aero requests: decoded from their own header, never trusted from a declared type or size, and bounded
   per provider at the most tokens the provider can bill for them. */
import type { Provider } from './catalogue';

export type ImageType = 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp';
/** Per request: how many images, and how big each may be. */
export const IMAGE_LIMITS = { count: 20, maxSide: 8000, maxPixels: 40_000_000 } as const;
const sized = (type: ImageType, width: number, height: number) => (width > 0 && height > 0 ? { type, width, height } : null);

/** An image's type and pixel size from its header (PNG, JPEG, GIF, WebP), or null when it is none of those. */
export function imageInfo(base64: string): { type: ImageType; width: number; height: number } | null {
  let bytes: Uint8Array;
  // The headers are near the start: decode the first 256 KiB (a whole number of base64 quads).
  try { bytes = Uint8Array.from(atob(base64.slice(0, 349_524)), c => c.charCodeAt(0)); } catch { return null; }
  const u16 = (i: number) => (bytes[i]! << 8) | bytes[i + 1]!;
  const u32 = (i: number) => ((bytes[i]! << 24) >>> 0) + (bytes[i + 1]! << 16) + (bytes[i + 2]! << 8) + bytes[i + 3]!;
  const le16 = (i: number) => bytes[i]! | (bytes[i + 1]! << 8);
  const le24 = (i: number) => bytes[i]! | (bytes[i + 1]! << 8) | (bytes[i + 2]! << 16);
  if (bytes.length >= 24 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return sized('image/png', u32(16), u32(20));
  if (bytes.length >= 10 && bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return sized('image/gif', le16(6), le16(8));
  if (bytes.length >= 30 && bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[8] === 0x57 && bytes[9] === 0x45) {
    const chunk = String.fromCharCode(bytes[12]!, bytes[13]!, bytes[14]!, bytes[15]!);
    if (chunk === 'VP8X') return sized('image/webp', le24(24) + 1, le24(27) + 1);
    if (chunk === 'VP8 ') return sized('image/webp', le16(26) & 0x3fff, le16(28) & 0x3fff);
    if (chunk === 'VP8L') { const b = bytes; return sized('image/webp', 1 + (((b[22]! & 0x3f) << 8) | b[21]!), 1 + (((b[24]! & 0x0f) << 10) | (b[23]! << 2) | ((b[22]! & 0xc0) >> 6))); }
    return null;
  }
  if (bytes.length >= 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    for (let i = 2; i + 9 < bytes.length;) {
      if (bytes[i] !== 0xff) { i++; continue; }
      const marker = bytes[i + 1]!;
      if (marker === 0xff) { i++; continue; }
      if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { i += 2; continue; }
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) return sized('image/jpeg', u16(i + 7), u16(i + 5));
      i += 2 + u16(i + 2);
    }
  }
  return null;
}


/** Why an image can't be sent, or null when it can: decodable, a supported type, within the size limits. */
export function imageProblem(base64: string): 'undecodable' | 'too_large' | null {
  const info = imageInfo(base64);
  if (!info) return 'undecodable';
  if (info.width > IMAGE_LIMITS.maxSide || info.height > IMAGE_LIMITS.maxSide || info.width * info.height > IMAGE_LIMITS.maxPixels) return 'too_large';
  return null;
}

/** Scales (w, h) down (never up) so neither side passes `side`. */
const fit = (w: number, h: number, side: number) => { const k = Math.min(1, side / Math.max(w, h)); return [Math.ceil(w * k), Math.ceil(h * k)] as const; };

/** The most input tokens a provider can bill for an image, from its decoded size (null when it doesn't decode).
    - Anthropic: about w*h/750 after its own downscale (long edge at most 1,568 on most models; 2,576 is assumed,
      the largest any current model takes, so the bound holds for all).
    - OpenAI at detail high: fit in 2048x2048, shortest side to 768, then 85 + 170 per 512-pixel tile; and never less
      than one token per 32-pixel patch of the image fitted in 2048 (the patch-based models). */
export function imageTokens(provider: Provider, base64: string): number | null {
  const info = imageInfo(base64);
  if (!info) return null;
  if (provider === 'anthropic') {
    const [w, h] = fit(info.width, info.height, 2576);
    return Math.ceil((w * h) / 750) + 64;
  }
  let [w, h] = fit(info.width, info.height, 2048);
  const short = Math.min(w, h);
  if (short > 768) { const k = 768 / short; w = Math.ceil(w * k); h = Math.ceil(h * k); }
  const tiles = Math.ceil(w / 512) * Math.ceil(h / 512);
  const [pw, ph] = fit(info.width, info.height, 2048);
  const patches = Math.ceil(pw / 32) * Math.ceil(ph / 32);
  return Math.max(85 + 170 * tiles, patches) + 64;
}
