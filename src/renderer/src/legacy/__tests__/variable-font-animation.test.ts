import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';

describe('variable font animation timing', () => {
  it('preserves linear interpolation when replacing axis keyframes', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ dur: 5 }); PM.time = 0;
    const layer = PM.mkLayer('text', { d: { 'fontAxis.wdth': PM.P(75) } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
    expect(PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path: 'c.fontAxis.wdth', keyframes: [
      { time: 0, value: 75, ease: 'linear' }, { time: 4, value: 125, ease: 'linear' }
    ] }).ok).toBe(true);
    expect(PM.resolveContent(layer, 1)['fontAxis.wdth']).toBeCloseTo(87.5);
    expect(PM.resolveContent(layer, 2)['fontAxis.wdth']).toBeCloseTo(100);
    expect(PM.resolveContent(layer, 3)['fontAxis.wdth']).toBeCloseTo(112.5);
  });
});
