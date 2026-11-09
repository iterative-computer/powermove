import { describe, expect, it } from 'vitest';
import { environmentProperties, terrainSchema, materialOptionsSchema } from '../../renderer/src/core/scene3d/environment-data';
import { domeAngles, domePoint, environmentKind, sourceFromKey, sourceKey, SKY_PRESETS, SKY_PRESET_VALUES } from './environment-ui';

describe('environment UI helpers', () => {
  it('places the sun on the dome and reads it back', () => {
    for (const [azimuth, elevation] of [[0, 45], [90, 10], [-60, 6], [170, 70]] as const) {
      const p = domePoint(azimuth, elevation, 46);
      const back = domeAngles(p.x, p.y, 46);
      expect(back.elevation).toBeCloseTo(elevation, 0);
      expect(((back.azimuth - azimuth) % 360 + 540) % 360 - 180).toBeCloseTo(0, 0);
    }
    expect(domePoint(0, 90, 46)).toEqual({ x: 0, y: 0 });
    expect(domePoint(0, 0, 46).y).toBeCloseTo(46);
  });
  it('keeps below-horizon drags inside the outer ring', () => {
    expect(domeAngles(0, 500, 46).elevation).toBe(-18);
  });
  it('round-trips environment sources', () => {
    for (const source of ['asset_1', { compositionId: 'c1' }, { layerId: 'l1' }]) expect(sourceFromKey(sourceKey(source))).toEqual(source);
    expect(sourceKey(null)).toBe('');
  });
  it('names layer kinds and covers every sky preset', () => {
    expect(environmentKind({ terrain: {} })).toBe('terrain');
    expect(environmentKind({ material: {} })).toBeNull();
    for (const preset of SKY_PRESETS) expect(SKY_PRESET_VALUES[preset.id]).toBeDefined();
  });
  it('gives Timeline rows plain names', () => {
    const terrain = terrainSchema.parse({ regions: [{ p: {} }] });
    const rows = environmentProperties({ terrain, materialOptions: materialOptionsSchema.parse({}) }).map(r => `${r.group}:${r.label}`);
    expect(rows).toContain('Terrain:Feature Size');
    expect(rows).toContain('Ground Cover 1:Steeper Than');
    expect(rows).toContain('Material Options:Casts Shadows');
  });
});
