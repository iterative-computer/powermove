import { describe, expect, it } from 'vitest';

import { DEFAULT_PROJECT_COLOR, hasProjectColor, nextProjectColor, readProjectColor } from './project-color';

describe('project color settings', () => {
  it('reads absent and partial settings as the default (migrated projects)', () => {
    expect(readProjectColor({})).toEqual(DEFAULT_PROJECT_COLOR);
    expect(readProjectColor({ color: null })).toEqual(DEFAULT_PROJECT_COLOR);
    expect(readProjectColor({ color: { linearBlending: true } })).toEqual({ ...DEFAULT_PROJECT_COLOR, linearBlending: true });
    expect(hasProjectColor({})).toBe(false);
    expect(hasProjectColor({ color: { ...DEFAULT_PROJECT_COLOR } })).toBe(false);
    expect(hasProjectColor({ color: { precision: 'float32' } })).toBe(true);
  });

  it('writes the field only away from the default, and reports no-op changes', () => {
    expect(nextProjectColor({}, { linearBlending: false })).toBeNull();
    expect(nextProjectColor({}, { linearBlending: true })).toEqual({ ...DEFAULT_PROJECT_COLOR, linearBlending: true });
    const linear = { color: { ...DEFAULT_PROJECT_COLOR, linearBlending: true } };
    expect(nextProjectColor(linear, { precision: 'float32' })).toEqual({ ...DEFAULT_PROJECT_COLOR, linearBlending: true, precision: 'float32' });
    expect(nextProjectColor(linear, { linearBlending: false })).toBeUndefined();
  });
});
