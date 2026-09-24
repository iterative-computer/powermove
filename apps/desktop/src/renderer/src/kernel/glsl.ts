/*
 * GLSL generation for kernel-registered effects and transitions.
 *
 * Extensions declare params by NAME (`{ k: 'amount' }`) and write shader bodies
 * against `u_amount`. The legacy compositor still binds uniforms POSITIONALLY
 * (`u_p0` for numbers/toggles, `u_c0` for colors), so every generated shader
 * also emits `#define u_p<i> u_<k>` aliases — old-style bodies copied out of
 * gl/shaders.ts keep compiling unchanged against the same param list.
 *
 * `rawShader: true` opts out entirely: the frag text is passed through as-is.
 */
import type { EffectDefinition, EffectParamDefinition, TransitionDefinition } from './api';

export const EFFECT_ID = /^[a-zA-Z][a-zA-Z0-9._-]{0,63}$/;
export const PARAM_KEY = /^[a-z][a-zA-Z0-9]*$/;
export const MAX_PASSES = 8;
export const MAX_PARAMS = 32;
export const MAX_FRAG_BYTES = 64 * 1024;

export interface LegacyFx {
  label: string;
  group: string;
  passes: number;
  params: EffectParamDefinition[];
  frag: string;
  keepOrig: boolean;
}

export interface LegacyTransition {
  label: string;
  group: string;
  params: EffectParamDefinition[];
  frag: string;
}

const isColor = (param: EffectParamDefinition): boolean => param.type === 'color';

function validateParams(params: unknown, what: string): EffectParamDefinition[] {
  if (!Array.isArray(params)) throw new Error(`${what}: "params" must be an array`);
  if (params.length > MAX_PARAMS) throw new Error(`${what}: too many params (${params.length} > ${MAX_PARAMS})`);
  const seen = new Set<string>();
  for (const param of params as EffectParamDefinition[]) {
    if (!param || typeof param !== 'object') throw new Error(`${what}: each param must be an object`);
    if (typeof param.k !== 'string' || !PARAM_KEY.test(param.k)) throw new Error(`${what}: invalid param key "${String(param?.k)}" (expected ${PARAM_KEY})`);
    if (seen.has(param.k)) throw new Error(`${what}: duplicate param key "${param.k}"`);
    seen.add(param.k);
  }
  return params as EffectParamDefinition[];
}

function validateFrag(frag: unknown, what: string): string {
  if (typeof frag !== 'string' || frag.length === 0) throw new Error(`${what}: "frag" must be a non-empty string`);
  if (frag.length > MAX_FRAG_BYTES) throw new Error(`${what}: shader is too large (${frag.length} > ${MAX_FRAG_BYTES} bytes)`);
  return frag;
}

export function validateEffect(def: EffectDefinition): EffectDefinition {
  if (!def || typeof def !== 'object') throw new Error('effect: definition must be an object');
  const what = `effect "${String(def.id)}"`;
  if (typeof def.id !== 'string' || !EFFECT_ID.test(def.id)) throw new Error(`effect: invalid id "${String(def.id)}"`);
  if (typeof def.label !== 'string' || !def.label) throw new Error(`${what}: "label" is required`);
  if (typeof def.group !== 'string' || !def.group) throw new Error(`${what}: "group" is required`);
  const passes = def.passes ?? 1;
  if (!Number.isInteger(passes) || passes < 1 || passes > MAX_PASSES) throw new Error(`${what}: "passes" must be an integer 1..${MAX_PASSES}`);
  validateParams(def.params, what);
  validateFrag(def.frag, what);
  return def;
}

export function validateTransition(def: TransitionDefinition): TransitionDefinition {
  if (!def || typeof def !== 'object') throw new Error('transition: definition must be an object');
  const what = `transition "${String(def.id)}"`;
  if (typeof def.id !== 'string' || !EFFECT_ID.test(def.id)) throw new Error(`transition: invalid id "${String(def.id)}"`);
  if (typeof def.label !== 'string' || !def.label) throw new Error(`${what}: "label" is required`);
  validateParams(def.params, what);
  validateFrag(def.frag, what);
  return def;
}

/** `uniform` declarations for the named params, in declaration order. */
export function paramDeclarations(params: EffectParamDefinition[]): string[] {
  return params.map((param) => `uniform ${isColor(param) ? 'vec3' : 'float'} u_${param.k};`);
}

/**
 * Value a non-color param contributes to its `float` uniform. Toggles are
 * declared as floats and bind as 0/1 — GLSL ES 3.00 has `bool`, but the legacy
 * compositor's uniform setter only speaks float/vec3/int.
 */
export function paramFloat(param: EffectParamDefinition, value: unknown): number {
  if (param.type === 'toggle') return value ? 1 : 0;
  const n = Number(value);
  return Number.isFinite(n) ? n : 0;
}

/** `#define u_p<i>|u_c<i> u_<k>` aliases keeping positional shader bodies valid. */
export function paramAliases(params: EffectParamDefinition[]): string[] {
  return params.map((param, index) => `#define ${isColor(param) ? 'u_c' : 'u_p'}${index} u_${param.k}`);
}

/** Declarations the preamble already provides are skipped (GLSL forbids redeclaration). */
function unlessDeclared(pre: string, name: string, line: string): string[] {
  return new RegExp(`\\bu_${name}\\b`).test(pre) ? [] : [line];
}

function assemble(pre: string, lines: string[], frag: string): string {
  const head = lines.filter(Boolean).join('\n');
  return `${pre}\n${head}\nvoid main(){\n${frag}\n}\n`;
}

/** Build the legacy `PM.FX[id]` shape from a kernel effect definition. */
export function toLegacyFx(def: EffectDefinition, pre: string): LegacyFx {
  validateEffect(def);
  const params = def.params;
  const passes = def.passes ?? 1;
  const keepOrig = def.keepOrig === true;
  const frag = def.rawShader
    ? def.frag
    : assemble(
        pre,
        [
          ...paramDeclarations(params),
          ...unlessDeclared(pre, 'pass', 'uniform int u_pass;'),
          ...(keepOrig ? unlessDeclared(pre, 'orig', 'uniform sampler2D u_orig;') : []),
          ...paramAliases(params)
        ],
        def.frag
      );
  return { label: def.label, group: def.group, passes, params, frag, keepOrig };
}

/** Build the legacy transition shape from a kernel transition definition. */
export function toLegacyTransition(def: TransitionDefinition, pre: string): LegacyTransition {
  validateTransition(def);
  const params = def.params;
  const frag = def.rawShader
    ? def.frag
    : assemble(
        pre,
        [
          ...unlessDeclared(pre, 'from', 'uniform sampler2D u_from;'),
          ...unlessDeclared(pre, 'to', 'uniform sampler2D u_to;'),
          ...unlessDeclared(pre, 'prog', 'uniform float u_prog;'),
          ...paramDeclarations(params),
          ...paramAliases(params)
        ],
        def.frag
      );
  return { label: def.label, group: def.group ?? 'Transitions', params, frag };
}
