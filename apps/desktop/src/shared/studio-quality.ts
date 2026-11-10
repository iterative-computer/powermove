import presets from './studio-quality-presets.json';
import { z } from 'zod';

export const STUDIO_QUALITY_PRESETS = Object.freeze({draft:Object.freeze(presets.draft),preview:Object.freeze(presets.preview),production:Object.freeze(presets.production),final:Object.freeze(presets.final)});
export type StudioQualityPreset = keyof typeof STUDIO_QUALITY_PRESETS;
const preset = z.enum(['draft', 'preview', 'production', 'final', 'custom']);
const fields = {
  noiseThreshold: z.number().finite().min(0).max(1),
  minSamples: z.number().int().min(1).max(4096), maxSamples: z.number().int().min(1).max(4096),
  timeLimitSeconds: z.number().finite().min(0).max(3600), maxBounces: z.number().int().min(1).max(32),
  caustics: z.boolean(), denoise: z.boolean(), motionBlurSamples: z.number().int().min(1).max(64),
};
const input = z.union([preset, z.object({ preset: preset.optional(), ...Object.fromEntries(Object.entries(fields).map(([k,v]) => [k,v.optional()])) }).strict()]);
export type StudioQuality = { preset: StudioQualityPreset | 'custom'; noiseThreshold: number; minSamples: number; maxSamples: number; timeLimitSeconds: number; maxBounces: number; caustics: boolean; denoise: boolean; motionBlurSamples: number };
export type StudioQualityInput = StudioQuality['preset'] | Partial<StudioQuality>;

export function parseStudioQualityInput(raw: unknown): StudioQualityInput {
  const value=input.parse(raw) as StudioQualityInput;
  if(typeof value!=='string' && value.minSamples!==undefined && value.maxSamples!==undefined && value.minSamples>value.maxSamples) throw new Error('Minimum samples must not exceed maximum samples');
  return value;
}

/** One resolver for document migration, guarded edits, capture and export.
 * A preset starts from its defaults; sparse custom patches retain other values.
 * Zero threshold disables error stopping; zero seconds disables the time cap. */
export function resolveStudioQuality(raw: unknown = 'production', base?: StudioQuality): StudioQuality {
  const parsed = parseStudioQualityInput(raw), patch: Partial<StudioQuality> = typeof parsed === 'string' ? { preset: parsed } : parsed;
  const name = patch.preset ?? base?.preset ?? 'production';
  const defaults = patch.preset && name !== 'custom' ? STUDIO_QUALITY_PRESETS[name] : base ?? STUDIO_QUALITY_PRESETS.production;
  const result = { ...defaults, ...patch, preset: name } as StudioQuality;
  if (result.minSamples > result.maxSamples) throw new Error('Minimum samples must not exceed maximum samples');
  if (name !== 'custom' && Object.entries(STUDIO_QUALITY_PRESETS[name]).some(([key,value]) => result[key as keyof StudioQuality] !== value)) result.preset = 'custom';
  return result;
}
export function migrateStudioQuality(raw: unknown): StudioQuality {
  try { return resolveStudioQuality(raw ?? 'production'); } catch { return resolveStudioQuality('production'); }
}
/** JSON tool schema uses the same bounds as document validation. */
export const studioQualityToolSchema = { anyOf: [
  { type: 'string', enum: ['draft','preview','production','final','custom'] },
  { type: 'object', additionalProperties: false, properties: {
    preset: { type: 'string', enum: ['draft','preview','production','final','custom'] },
    noiseThreshold: { type: 'number', minimum: 0, maximum: 1 },
    minSamples: { type: 'integer', minimum: 1, maximum: 4096 }, maxSamples: { type: 'integer', minimum: 1, maximum: 4096 },
    timeLimitSeconds: { type: 'number', minimum: 0, maximum: 3600 }, maxBounces: { type: 'integer', minimum: 1, maximum: 32 },
    caustics: { type: 'boolean' }, denoise: { type: 'boolean' }, motionBlurSamples: { type: 'integer', minimum: 1, maximum: 64 },
  } },
] };
