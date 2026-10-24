import { describe, expect, it } from 'vitest';

import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import type { PMRegistry } from '../registry';
import { install } from './shaders';
import { install as installCompositor } from './compositor';

function shaderRegistry(): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    uid: (prefix: string) => `${prefix}${++nextId}`,
    P: (value: any) => ({ v: value, kf: [], expr: '' }),
  };
  install(PM);
  for (const definition of EFFECTS) PM.Kernel.registerEffect('effects-basic', definition);
  return PM;
}

describe('legacy shader registry install', () => {
  it('exposes extension effect definitions and creates their parameter values', () => {
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

  it('retires completed frames and prefixes when an effect is disabled, restored or replaced', () => {
    const PM = shaderRegistry();
    installCompositor(PM);
    const definition = { id: 'cachepaint', label: 'Cached paint', group: 'Test', params: [], frag: 'o = texture(u_tex, v_st);' };
    const remember = () => {
      PM.GL.previewFrames.put('frame', { bytes: 16 }, 64);
      PM.GL.previewFrames.put('prefix', { bytes: 16 }, 64);
      PM.GL.progs.set('fx:cachepaint', {});
      expect(PM.GL.previewFrames.count).toBe(2);
    };
    const first = PM.Kernel.registerEffect('cache-provider', definition);
    const layerEffect = PM.mkEffect('cachepaint');
    remember(); first.dispose();
    expect(PM.GL.previewFrames.count).toBe(0);
    expect(PM.GL.progs.has('fx:cachepaint')).toBe(false);
    // A picture rendered while the provider is off must be retired on add.
    remember();
    const restored = PM.Kernel.registerEffect('cache-provider', definition);
    expect(PM.GL.previewFrames.count).toBe(0);
    expect(PM.GL.progs.has('fx:cachepaint')).toBe(false);
    expect(layerEffect.on).toBe(true);
    remember();
    const override = PM.Kernel.registerEffect('cache-override', { ...definition, frag: 'o = vec4(1.);' });
    expect(PM.GL.previewFrames.count).toBe(0);
    expect(PM.GL.progs.has('fx:cachepaint')).toBe(false);
    remember(); override.dispose();
    expect(PM.GL.previewFrames.count).toBe(0);
    expect(PM.GL.progs.has('fx:cachepaint')).toBe(false);
    restored.dispose();
  });
});
