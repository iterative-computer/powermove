import { describe, expect, it } from 'vitest';
import { staticExportPlan } from './static-export';

const solid = () => ({ type: 'solid', from: 1, dur: 2, p: { opacity: { v: 100, kf: [] } } });

describe('static export planning', () => {
  it('retains a background and splits pictures at layer entrances and exits', () => {
    const background = staticExportPlan({ layers: [] }, false)!;
    expect(background(0)).toBe(background(10));
    const plan = staticExportPlan({ layers: [solid()] }, false)!;
    expect(plan(.5)).not.toBe(plan(1));
    expect(plan(1)).toBe(plan(2.9));
    expect(plan(2.9)).not.toBe(plan(3));
  });

  it.each([
    { type: 'video' }, { type: 'image' }, { type: 'precomp' }, { type: 'shader' }, { type: 'extension' },
    { fx: [{}] }, { transitionIn: {} }, { threeD: true }, { masks: [{}] }, { matteSource: 'other' },
    { d: { animators: [{}] } }, { p: { rotation: { expr: 'time' } } },
    { p: { rotation: { kf: [{ t: 0, v: 0 }, { t: 1, v: 0, spring: true }] } } },
  ])('keeps temporal or unsupported artwork on the normal path: %j', change => {
    expect(staticExportPlan({ layers: [{ ...solid(), ...change }] }, false)).toBeNull();
  });

  it('keeps audio independent and renders each blur frame around a boundary', () => {
    const plan = staticExportPlan({ fps: 30, layers: [{ ...solid(), mblur: true }, { type: 'audio' }] }, true)!;
    expect(plan(.99)).not.toBe(plan(1));
    expect(plan(1)).not.toBe(plan(1.01));
    expect(plan(1.1)).toBe(plan(2));
  });
});
