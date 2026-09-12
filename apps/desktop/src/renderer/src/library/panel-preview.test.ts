import { describe, expect, it } from 'vitest';
import { panelBelongsInLibrary, panelPreviewSize } from './panel-preview';

describe('panel Library presentation', () => {
  it('uses definition-owned preview geometry', () => {
    expect(panelPreviewSize({ library: { width: 800, height: 440 }, size: 120 })).toEqual({ width: 800, height: 440 });
  });

  it('has a stable fallback independent of workspace geometry', () => {
    expect(panelPreviewSize({ size: 240 })).toEqual({ width: 360, height: 240 });
    expect(panelPreviewSize({})).toEqual({ width: 360, height: 320 });
  });

  it('allows non-panel chrome to opt out', () => {
    expect(panelBelongsInLibrary({ library: false })).toBe(false);
    expect(panelBelongsInLibrary({})).toBe(true);
  });
});
