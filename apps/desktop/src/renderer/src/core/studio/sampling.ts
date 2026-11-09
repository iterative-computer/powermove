/** Legacy project values (often 8–16 spp) are quality hints, not enough input
 * for denoising. Each shutter image needs its own useful sample budget. */
export const STUDIO_FINAL_SAMPLES = 1024;
export const STUDIO_STILL_SAMPLES = 256;
export function finalSampleLimit(requested: unknown, shutterSamples = 1): number {
  const value = Number(requested ?? STUDIO_FINAL_SAMPLES);
  const samples = Number.isFinite(value) ? Math.max(512, value) : STUDIO_FINAL_SAMPLES;
  const shutter = Number.isFinite(shutterSamples) ? Math.max(1, Math.ceil(shutterSamples)) : 1;
  return Math.min(1024, Math.max(128, Math.ceil(samples / shutter)));
}
