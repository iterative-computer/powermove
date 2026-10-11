// Procedural corpus textures (256², RGBA8, top row first). Deterministic by construction.

export type Image8 = { width: number; height: number; pixels: Uint8Array };

function make(size: number, fn: (u: number, v: number, x: number, y: number) => [number, number, number, number]): Image8 {
  const pixels = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const [r, g, b, a] = fn((x + 0.5) / size, (y + 0.5) / size, x, y);
    const i = (y * size + x) * 4;
    pixels[i] = Math.round(Math.min(255, Math.max(0, r)));
    pixels[i + 1] = Math.round(Math.min(255, Math.max(0, g)));
    pixels[i + 2] = Math.round(Math.min(255, Math.max(0, b)));
    pixels[i + 3] = Math.round(Math.min(255, Math.max(0, a)));
  }
  return { width: size, height: size, pixels };
}

/** Color checker with a tinted cell grid; strong high frequencies for sampling tests. */
export const checker = () => make(256, (u, v, x, y) => {
  const cell = ((x >> 5) + (y >> 5)) & 1;
  const tint = [40 + 180 * u, 60 + 120 * (1 - v), 90 + 100 * ((x >> 5) % 3) / 2];
  return cell ? [tint[0]!, tint[1]!, tint[2]!, 255] : [235, 228, 210, 255];
});

/**
 * Orientation chart: top-left red, top-right green, bottom-left blue, bottom-right
 * yellow, with a white arrow pointing to the image top. Any flip or rotation is visible.
 */
export const orientation = () => make(256, (u, v) => {
  const arrow = (Math.abs(u - 0.5) < 0.06 && v > 0.3 && v < 0.75) || (v >= 0.15 && v <= 0.32 && Math.abs(u - 0.5) < (v - 0.15) * 0.9);
  if (arrow) return [255, 255, 255, 255];
  if (v < 0.5) return u < 0.5 ? [220, 40, 40, 255] : [40, 190, 60, 255];
  return u < 0.5 ? [40, 70, 220, 255] : [235, 210, 40, 255];
});

/** Tangent-space normal map: a grid of hemispherical bumps (OpenGL convention, +Y up). */
export const bumps = () => make(256, (u, v) => {
  const cu = (u * 4) % 1 - 0.5, cv = (v * 4) % 1 - 0.5, r = Math.hypot(cu, cv);
  let nx = 0, ny = 0;
  if (r < 0.4) { const s = Math.sin((r / 0.4) * Math.PI) * 0.7; nx = (cu / Math.max(r, 1e-6)) * s; ny = -(cv / Math.max(r, 1e-6)) * s; }
  const nz = Math.sqrt(Math.max(0, 1 - nx * nx - ny * ny));
  return [(nx * 0.5 + 0.5) * 255, (ny * 0.5 + 0.5) * 255, (nz * 0.5 + 0.5) * 255, 255];
});

/** Packed roughness (G) and metalness (B), glTF style; R carries ambient occlusion. */
export const orm = () => make(256, (u, v) => {
  const ao = 255 * (0.35 + 0.65 * Math.min(1, Math.hypot(u - 0.5, v - 0.5) * 2.2));
  return [ao, 255 * (0.1 + 0.85 * u), ((Math.floor(v * 6) & 1) ? 230 : 20), 255];
});

export const roughness = () => make(256, u => { const g = 255 * (0.08 + 0.9 * u); return [g, g, g, 255]; });
export const metalness = () => make(256, (_u, v) => { const m = (Math.floor(v * 6) & 1) ? 240 : 15; return [m, m, m, 255]; });
export const ao = () => make(256, (u, v) => { const a = 255 * (0.3 + 0.7 * Math.min(1, Math.hypot(u - 0.5, v - 0.5) * 2.4)); return [a, a, a, 255]; });

/** Emissive dots on black. */
export const emissive = () => make(256, (u, v) => {
  const on = Math.hypot((u * 8) % 1 - 0.5, (v * 8) % 1 - 0.5) < 0.22;
  return on ? [255, 150 + 100 * u, 60, 255] : [0, 0, 0, 255];
});

/** RGBA leaf mask for alpha cutoff and blending tests (soft edges, holes). */
export const cutout = () => make(256, (u, v) => {
  const d = Math.hypot(u - 0.5, v - 0.5);
  const holes = Math.hypot((u * 5) % 1 - 0.5, (v * 5) % 1 - 0.5) < 0.18;
  const a = holes ? 0 : 255 * Math.min(1, Math.max(0, (0.48 - d) * 25));
  return [60 + 150 * u, 170, 70 + 120 * v, a];
});

/** Iridescence thickness / bump height ramp. */
export const ramp = () => make(256, (u, v) => { const g = 255 * (0.5 + 0.5 * Math.sin((u + v) * Math.PI * 6)); return [g, g, g, 255]; });
