import { describe, expect, it } from 'vitest';

import { inspectSvgExport } from './svg-compatibility';

const channel = (v: unknown) => ({ v, kf: [], expr: null });
const group = (patch: Record<string, unknown> = {}) => ({
  id: 'group', name: 'Group', type: 'group', blend: 'normal', mblur: false,
  p: {}, fx: [], masks: [], transitionIn: null, transitionOut: null,
  ...patch,
});

describe('SVG compatibility for groups', () => {
  it('allows fixed default compositing properties', () => {
    expect(inspectSvgExport({ layers: [group({ blend: channel('normal'), mblur: channel(false) })] }))
      .toEqual({ supported: true, reasons: [] });
  });

  it.each([
    ['effects', { fx: [{ type: 'blur', on: true }] }],
    ['masks', { masks: [{}] }],
    ['blend', { blend: 'multiply' }],
    ['motion blur', { mblur: true }],
    ['matte', { matteSource: 'shape' }],
    ['transitions', { transitionIn: { type: 'wipe' } }],
  ])('routes group %s through WebGL instead of dropping it from SVG', (_feature, patch) => {
    const result = inspectSvgExport({ layers: [group(patch)] });
    expect(result.supported).toBe(false);
    expect(result.reasons).toHaveLength(1);
    expect(result.reasons[0]).toMatch(/requires? WebGL/);
  });
});
