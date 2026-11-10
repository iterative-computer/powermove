import { describe, expect, it } from 'vitest';
import { describeStudioQuality, STUDIO_QUALITY_CHOICES, STUDIO_QUALITY_FIELDS } from './studio-quality-ui';
import { STUDIO_QUALITY_PRESETS } from './studio-quality';

describe('render quality descriptions', () => {
  it('names every preset and Custom in order, with a plain summary', () => {
    expect(STUDIO_QUALITY_CHOICES.map(c => c.label)).toEqual(['Draft', 'Preview', 'Production', 'Final', 'Custom']);
    for (const choice of STUDIO_QUALITY_CHOICES) expect(choice.summary.length).toBeGreaterThan(10);
  });
  it('estimates from the time limit per frame and for a whole export', () => {
    expect(describeStudioQuality('production')).toMatchObject({ label: 'Production', perFrame: 'Up to about 3 s per frame', total: null });
    expect(describeStudioQuality('final', 300).total).toBe('Up to about 1.7 h for 300 frames');
    expect(describeStudioQuality('draft', 30).perFrame).toBe('Up to about 0.5 s per frame');
    expect(describeStudioQuality({ preset: 'custom', timeLimitSeconds: 0, maxSamples: 64, minSamples: 8 }).perFrame).toBe('No time limit · 64 samples per frame');
  });
  it('reads unknown input as Production rather than failing', () => {
    expect(describeStudioQuality('nonsense').label).toBe('Production');
  });
  it('offers a Custom field for every saved setting', () => {
    expect(STUDIO_QUALITY_FIELDS.map(f => f.key).sort()).toEqual(Object.keys(STUDIO_QUALITY_PRESETS.production).sort());
  });
});
