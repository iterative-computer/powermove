import { describe, expect, it } from 'vitest';
import type { EffectDefinition, EffectParamDefinition, TransitionDefinition } from './api';
import { paramAliases, paramDeclarations, paramFloat, toLegacyFx, toLegacyTransition, validateEffect } from './glsl';

/** Stand-in for the real gl/shaders.ts preamble — it already declares u_prog. */
const PRE = ['#version 300 es', 'precision highp float;', 'uniform float u_time;', 'uniform float u_prog;', 'out vec4 o;'].join('\n');

const params: EffectParamDefinition[] = [
  { k: 'amount', label: 'Amount', def: 24, min: 0, max: 400, step: 0.5, unit: 'px' },
  { k: 'tint', label: 'Tint', def: '#ff0066', type: 'color' },
  { k: 'invert', label: 'Invert', def: false, type: 'toggle' }
];

const effect = (over: Partial<EffectDefinition> = {}): EffectDefinition => ({
  id: 'vhs',
  label: 'VHS',
  group: 'Stylize',
  params,
  frag: '  o = src(v_st) * u_amount;',
  ...over
});

describe('param declarations and aliases', () => {
  it('declares floats for numbers and toggles and vec3 for colors', () => {
    expect(paramDeclarations(params)).toEqual(['uniform float u_amount;', 'uniform vec3 u_tint;', 'uniform float u_invert;']);
  });

  it('aliases positional names by kind, indexed across the whole param list', () => {
    expect(paramAliases(params)).toEqual(['#define u_p0 u_amount', '#define u_c1 u_tint', '#define u_p2 u_invert']);
  });

  it('binds toggles as 0/1 floats and coerces junk numbers to 0', () => {
    const toggle = params[2]!;
    const number = params[0]!;
    expect(paramFloat(toggle, true)).toBe(1);
    expect(paramFloat(toggle, false)).toBe(0);
    expect(paramFloat(number, '12.5')).toBe(12.5);
    expect(paramFloat(number, 'nope')).toBe(0);
  });
});

describe('toLegacyFx', () => {
  it('produces the legacy FX shape with generated uniforms, u_pass, and a main()', () => {
    const fx = toLegacyFx(effect({ passes: 2 }), PRE);

    expect(fx).toMatchObject({ label: 'VHS', group: 'Stylize', passes: 2, keepOrig: false, params });
    expect(fx.frag.startsWith(PRE)).toBe(true);
    expect(fx.frag).toContain('uniform float u_amount;');
    expect(fx.frag).toContain('uniform vec3 u_tint;');
    expect(fx.frag).toContain('uniform int u_pass;');
    expect(fx.frag).toContain('#define u_p0 u_amount');
    expect(fx.frag).toContain('#define u_c1 u_tint');
    expect(fx.frag).not.toContain('uniform sampler2D u_orig;');
    expect(fx.frag).toContain('void main(){\n  o = src(v_st) * u_amount;\n}');
  });

  it('defaults passes to 1 and adds u_orig only when keepOrig', () => {
    expect(toLegacyFx(effect(), PRE).passes).toBe(1);
    expect(toLegacyFx(effect({ keepOrig: true }), PRE).frag).toContain('uniform sampler2D u_orig;');
  });

  it('passes rawShader frag through untouched', () => {
    const raw = '#version 300 es\nvoid main(){ o = vec4(1.); }';
    const fx = toLegacyFx(effect({ rawShader: true, frag: raw }), PRE);
    expect(fx.frag).toBe(raw);
  });

  it('emits a shader with no param block when there are no params', () => {
    const fx = toLegacyFx(effect({ params: [], frag: 'o = vec4(0.);' }), PRE);
    expect(fx.frag).not.toContain('#define');
    expect(fx.frag).toContain('uniform int u_pass;');
  });

  it('validates the definition before generating', () => {
    expect(() => toLegacyFx(effect({ id: '' }), PRE)).toThrow(/invalid id/);
    expect(() => validateEffect(effect({ group: '' }))).toThrow(/group/);
    expect(() => validateEffect(effect({ params: [params[0]!, params[0]!] }))).toThrow(/duplicate param key/);
    expect(() => validateEffect(effect({ frag: '' }))).toThrow(/non-empty/);
  });
});

describe('toLegacyTransition', () => {
  const transition = (over: Partial<TransitionDefinition> = {}): TransitionDefinition => ({
    id: 'wipe',
    label: 'Wipe',
    params: [{ k: 'softness', label: 'Softness', def: 0.1, min: 0, max: 1 }],
    frag: '  o = mix(texture(u_from, v_st), texture(u_to, v_st), u_prog);',
    ...over
  });

  it('declares u_from/u_to and skips u_prog because the preamble already has it', () => {
    const out = toLegacyTransition(transition(), PRE);
    expect(out.label).toBe('Wipe');
    expect(out.group).toBe('Transitions');
    expect(out.frag).toContain('uniform sampler2D u_from;');
    expect(out.frag).toContain('uniform sampler2D u_to;');
    expect(out.frag.match(/uniform float u_prog;/g)).toHaveLength(1);
    expect(out.frag).toContain('uniform float u_softness;');
    expect(out.frag).toContain('#define u_p0 u_softness');
  });

  it('declares u_prog when the preamble does not', () => {
    const bare = '#version 300 es\nout vec4 o;';
    expect(toLegacyTransition(transition(), bare).frag).toContain('uniform float u_prog;');
  });

  it('honours group overrides and rawShader', () => {
    expect(toLegacyTransition(transition({ group: 'Custom' }), PRE).group).toBe('Custom');
    expect(toLegacyTransition(transition({ rawShader: true, frag: 'RAW' }), PRE).frag).toBe('RAW');
  });
});
