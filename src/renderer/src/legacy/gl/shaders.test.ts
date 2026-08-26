import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './shaders';

function shaderRegistry(): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    uid: (prefix: string) => `${prefix}${++nextId}`,
    P: (value: any) => ({ v: value, kf: [], expr: '' }),
  };
  install(PM);
  return PM;
}

describe('legacy shader registry install', () => {
  it('installs effect definitions and creates their parameter values', () => {
    const PM = shaderRegistry();
    const blur = PM.mkEffect('blur');
    blur.p.amount.v = 12;

    expect(Object.keys(blur.p)).toEqual(Array.from(PM.FX.blur.params, (parameter: any) => parameter.k));
    expect(blur.p.amount.v).toBe(12);
    expect(blur.type).toBe('blur');
    expect(PM.mkEffect('missing')).toBeNull();
  });

  it('keeps effect state mutable and parses inspector-ready uniform annotations', () => {
    const PM = shaderRegistry();
    const toggleEffect = PM.mkEffect('blur');
    toggleEffect.on = false;
    const defs = PM.parseUniforms(`uniform vec3 uTint; // @param #123456
uniform float uSpeed; // @param 0.5 0 3
uniform bool uEnabled; // @param true
uniform float iTime;`);

    expect(Object.keys(toggleEffect.p)).toEqual(Array.from(PM.FX.blur.params, (parameter: any) => parameter.k));
    expect(toggleEffect.on).toBe(false);
    expect(defs).toHaveLength(3);
    expect(defs[0]).toMatchObject({ name: 'uTint', control: 'color', def: '#123456' });
  });
});
