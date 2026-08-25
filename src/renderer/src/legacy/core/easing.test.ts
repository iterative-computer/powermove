import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './easing';

function easingRegistry(): PMRegistry {
  const PM: PMRegistry = {};
  install(PM);
  return PM;
}

describe('legacy easing install', () => {
  it('installs the preset contract and cached bezier solver', () => {
    const PM = easingRegistry();
    const linear = PM.Ease.bezier(0, 0, 1, 1);

    expect(PM.Ease.names).toContain('power');
    expect(PM.Ease.handles('power')).toEqual({ eo: [.62, .05], ei: [0, 1] });
    expect(PM.Ease.nameOf([.62, .05], [0, 1])).toBe('power');
    expect(PM.Ease.bezier(0, 0, 1, 1)).toBe(linear);
    expect([linear(0), linear(.25), linear(1)]).toEqual([0, .25, 1]);
  });

  it('keeps spring output deterministic and bounded at the start', () => {
    const PM = easingRegistry();

    expect(PM.Ease.spring(0)).toBe(0);
    expect(PM.Ease.spring(.5)).toBe(PM.Ease.spring(.5));
    expect(PM.Ease.springDuration()).toBeGreaterThan(0);
  });
});
