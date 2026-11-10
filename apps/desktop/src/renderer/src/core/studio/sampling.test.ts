import { expect, it } from 'vitest';
import { resolveStudioQuality, STUDIO_QUALITY_PRESETS, shutterQuality, finalSampleLimit } from './sampling';
import { migrateStudioQuality } from '../../../../shared/studio-quality';
it('resolves all presets and honors lower custom ceilings without a hidden floor', () => {
  for (const name of Object.keys(STUDIO_QUALITY_PRESETS) as (keyof typeof STUDIO_QUALITY_PRESETS)[]) {
    const q = resolveStudioQuality(name);
    expect(q).toEqual({preset:name,...STUDIO_QUALITY_PRESETS[name]});
    const split = shutterQuality(q, 2);
    expect(split.minSamples * 2).toBe(q.minSamples);
    expect(split.maxSamples * 2).toBe(q.maxSamples);
    expect(split.timeBudgetMs * 2).toBe(q.timeLimitSeconds * 1000);
  }
  expect(finalSampleLimit(16, 2)).toBe(8);
  expect(finalSampleLimit(NaN, NaN)).toBe(1024);
});
it('retains custom values, resets on a preset, and validates before mutation', () => {
  const custom = resolveStudioQuality({maxSamples:48}, resolveStudioQuality('preview'));
  expect(custom).toMatchObject({preset:'custom',minSamples:16,maxSamples:48});
  expect(resolveStudioQuality('final', custom)).toEqual(resolveStudioQuality('final'));
  for (const invalid of [{minSamples:80,maxSamples:16},{maxSamples:Infinity},{maxSamples:0},{motionBlurSamples:1.2},{timeLimitSeconds:-1},{caustics:1},{noiseThreshold:NaN},{bad:1}])
    expect(() => resolveStudioQuality(invalid)).toThrow();
  expect(migrateStudioQuality(undefined)).toEqual(resolveStudioQuality('production'));
  expect(migrateStudioQuality({noiseThreshold:NaN})).toEqual(resolveStudioQuality('production'));
});

it('keeps rounded shutter budgets below the ceiling and preserves tiny positive deadlines',()=>{
 const q=resolveStudioQuality({preset:'draft',maxSamples:5,timeLimitSeconds:.0001});
 const split=shutterQuality(q,2);expect(split.maxSamples*2).toBeLessThanOrEqual(5);expect(split.minSamples).toBeLessThanOrEqual(split.maxSamples);expect(split.timeBudgetMs).toBe(1);
});
