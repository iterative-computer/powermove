import { resolveStudioQuality, type StudioQuality } from '../../../../shared/studio-quality';
export { resolveStudioQuality, STUDIO_QUALITY_PRESETS } from '../../../../shared/studio-quality';
export const STUDIO_FINAL_SAMPLES = 1024;
export const STUDIO_STILL_SAMPLES = 256;
export function qualityForComposition(PM: any, composition = PM._renderComposition3D || PM.curComp?.() || PM.proj): StudioQuality {
  return resolveStudioQuality(PM._studioQualityOverride ?? composition?.studioQuality ?? 'production');
}
export function shutterQuality(quality: StudioQuality, shutterSamples = 1) {
  const shutter = Number.isFinite(shutterSamples) ? Math.max(1, Math.ceil(shutterSamples)) : 1;
  const maxSamples=Math.max(1,Math.floor(quality.maxSamples/shutter));
  return { ...quality,maxSamples,minSamples:Math.min(maxSamples,Math.max(1,Math.ceil(quality.minSamples/shutter))),
    timeBudgetMs:quality.timeLimitSeconds>0?Math.max(1,Math.round(quality.timeLimitSeconds*1000/shutter)):0 };
}
/** Kept for integrations using the old helper; explicit low budgets are honored. */
export function finalSampleLimit(requested: unknown, shutterSamples = 1): number {
  const value = Number(requested ?? STUDIO_FINAL_SAMPLES);
  return shutterQuality({ ...resolveStudioQuality('final'), maxSamples: Number.isFinite(value) ? Math.min(4096, Math.max(1, Math.ceil(value))) : STUDIO_FINAL_SAMPLES }, shutterSamples).maxSamples;
}
