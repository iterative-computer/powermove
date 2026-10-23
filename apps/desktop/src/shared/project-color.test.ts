import { describe, expect, it } from 'vitest';

import { DEFAULT_PROJECT_COLOR, hasProjectColor, migrateProjectColorDepth, nextProjectColor, readProjectColor } from './project-color';

describe('project color settings', () => {
  it('reads absent and partial settings as the default (migrated projects)', () => {
    expect(readProjectColor({})).toEqual(DEFAULT_PROJECT_COLOR);
    expect(readProjectColor({ color: null })).toEqual(DEFAULT_PROJECT_COLOR);
    expect(readProjectColor({ color: { linearBlending: true } })).toEqual({ ...DEFAULT_PROJECT_COLOR, linearBlending: true });
    expect(hasProjectColor({})).toBe(false);
    expect(hasProjectColor({ color: { ...DEFAULT_PROJECT_COLOR } })).toBe(false);
    expect(readProjectColor({ colorDepth: 8 }).precision).toBe('unorm8');
    expect(readProjectColor({ colorDepth: 16 }).precision).toBe('float16');
    expect(readProjectColor({ colorDepth: 32 }).precision).toBe('float32');
    expect(hasProjectColor({ colorDepth: 32 })).toBe(false);
    expect(hasProjectColor({ color: { precision: 'float32' } })).toBe(false);
  });

  it('writes the field only away from the default, and reports no-op changes', () => {
    expect(nextProjectColor({}, { linearBlending: false })).toBeNull();
    expect(nextProjectColor({}, { linearBlending: true })).toEqual({ mode: 'standard', linearBlending: true, blendDomain: 'hdrSafe' });
    const linear = { color: { ...DEFAULT_PROJECT_COLOR, linearBlending: true } };
    expect(nextProjectColor(linear, { linearBlending: false })).toBeUndefined();
  });

  it('moves old precision into Color depth without changing an explicit depth', () => {
    for (const [precision, expected] of [['float16', 16], ['float32', 32]] as const) {
      const source = { color: { mode: 'standard', precision, linearBlending: true } };
      const migrated = migrateProjectColorDepth(source);
      expect(migrated).toEqual({ colorDepth: expected, color: { mode: 'standard', linearBlending: true } });
      expect(source.color.precision).toBe(precision);
      expect(migrateProjectColorDepth(migrated)).toEqual(migrated);
    }
    for (const depth of [8, 16, 32] as const) {
      expect(migrateProjectColorDepth({ colorDepth: depth, color: { precision: 'float32' } }))
        .toEqual({ colorDepth: depth });
    }
    expect(migrateProjectColorDepth({})).toEqual({ colorDepth: 16 });
  });
});
