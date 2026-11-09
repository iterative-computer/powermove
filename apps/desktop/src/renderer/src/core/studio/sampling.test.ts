import { expect, it } from 'vitest';
import { finalSampleLimit, STUDIO_STILL_SAMPLES } from './sampling';

it('keeps old low-sample projects and shutter images above the denoising floor', () => {
  expect(finalSampleLimit(16, 2)).toBe(256);
  expect(finalSampleLimit(8, 8)).toBe(128);
  expect(finalSampleLimit(undefined, 2)).toBe(512);
  expect(STUDIO_STILL_SAMPLES).toBeGreaterThanOrEqual(128);
});
it('bounds quality work even for invalid or oversized saved settings', () => {
  for (const value of [NaN, Infinity, -10, 100000])
    expect(finalSampleLimit(value)).toBeGreaterThanOrEqual(128);
  expect(finalSampleLimit(100000)).toBe(1024);
  expect(finalSampleLimit(NaN, NaN)).toBe(1024);
});
