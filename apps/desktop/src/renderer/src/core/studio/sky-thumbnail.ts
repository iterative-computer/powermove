import { bakeSky, type StudioSky } from './environment';

/* Sky preset thumbnails from Studio's own sky model (bakeSky, shared with
 * pm-studio's environment.rs): a small view toward the sun, a little above
 * the horizon, with the ground below. One fixed exposure for every preset so
 * Night stays dark and Clear noon bright, as they render. */

export interface SkyThumbnailInput { elevation: number; azimuth?: number; haze: number; overcast: number; sunIntensity: number; intensity?: number }

const EXPOSURE = 2.2;
const srgb = (v: number) => { const c = Math.max(0, Math.min(1, v)); return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055; };
/** Filmic shoulder so the bright horizon and sun do not clip harshly. */
const tone = (v: number) => { const x = Math.max(0, v * EXPOSURE); return x / (1 + x); };

export function skyThumbnailPixels(input: SkyThumbnailInput, width = 48, height = 48): Uint8ClampedArray<ArrayBuffer> {
  const sky: StudioSky = { elevation: input.elevation, azimuth: 0, turbidity: input.haze, overcast: input.overcast, sun_intensity: input.sunIntensity, intensity: input.intensity ?? 1 };
  const map = bakeSky(sky, 160, 80);
  const at = (x: number, y: number) => map.pixels[Math.min(map.height - 1, Math.max(0, y)) * map.width + ((x % map.width) + map.width) % map.width]!;
  const out = new Uint8ClampedArray(new ArrayBuffer(width * height * 4));
  // View: 70° wide, centred 25° left of the sun (so the sun sits right of centre), looking 18° up.
  const fovX = (70 * Math.PI) / 180, fovY = fovX * (height / width), pitch0 = (12 * Math.PI) / 180, yaw0 = (-18 * Math.PI) / 180;
  const sunElevation = (input.elevation * Math.PI) / 180;
  const sunDir = [0, Math.sin(sunElevation), Math.cos(sunElevation)];
  const sunRadius = 0.05, glow = input.sunIntensity * (1 - input.overcast);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const yaw = yaw0 + ((x + 0.5) / width - 0.5) * fovX, pitch = pitch0 + (0.5 - (y + 0.5) / height) * fovY;
    const d = [Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(yaw) * Math.cos(pitch)];
    // Equirectangular lookup matching bakeSky's layout (phi from +Z toward +X).
    const theta = Math.acos(Math.max(-1, Math.min(1, d[1]!)));
    let phi = Math.atan2(d[0]!, d[2]!); if (phi < 0) phi += Math.PI * 2;
    // Bilinear, so the horizon has no steps.
    const fx = (phi / (Math.PI * 2)) * map.width - 0.5, fy = (theta / Math.PI) * map.height - 0.5;
    const x0 = Math.floor(fx), y0 = Math.floor(fy), tx = fx - x0, ty = fy - y0;
    const a = at(x0, y0), b = at(x0 + 1, y0), c0 = at(x0, y0 + 1), e = at(x0 + 1, y0 + 1);
    const mix = (k: 'x' | 'y' | 'z') => (a[k] * (1 - tx) + b[k] * tx) * (1 - ty) + (c0[k] * (1 - tx) + e[k] * tx) * ty;
    let rgb = [mix('x'), mix('y'), mix('z')];
    if (d[1]! >= 0 && glow > 0 && input.elevation > -2) {
      const cos = d[0]! * sunDir[0]! + d[1]! * sunDir[1]! + d[2]! * sunDir[2]!, angle = Math.acos(Math.max(-1, Math.min(1, cos)));
      const disc = angle < sunRadius ? 1 : Math.exp(-(((angle - sunRadius) / 0.06) ** 2)) * 0.35;
      rgb = rgb.map(v => v + disc * glow * 1.6);
    }
    const i = (y * width + x) * 4;
    out[i] = Math.round(srgb(tone(rgb[0]!)) * 255); out[i + 1] = Math.round(srgb(tone(rgb[1]!)) * 255); out[i + 2] = Math.round(srgb(tone(rgb[2]!)) * 255); out[i + 3] = 255;
  }
  return out;
}

const cache = new Map<string, string>();
/** A PNG data URL, or null where no canvas is available. */
export function skyThumbnail(input: SkyThumbnailInput, size = 48): string | null {
  const key = JSON.stringify([input, size]);
  const hit = cache.get(key); if (hit) return hit;
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas'); canvas.width = canvas.height = size;
  const ctx = canvas.getContext('2d'); if (!ctx) return null;
  ctx.putImageData(new ImageData(skyThumbnailPixels(input, size, size), size, size), 0, 0);
  const url = canvas.toDataURL('image/png'); cache.set(key, url); return url;
}
