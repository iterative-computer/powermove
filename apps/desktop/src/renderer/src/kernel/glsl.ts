import {referenceParam} from '../core/effect-inputs';
import { validEffectParameterDefault } from './effect-defaults';
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
import { effectVersion } from './effect-versions';

export const EFFECT_ID = /^[a-zA-Z][a-zA-Z0-9._-]{0,63}$/;
export const PARAM_KEY = /^[a-z][a-zA-Z0-9]*$/;
export const MAX_PASSES = 8;
export const MAX_FRAG_BYTES = 64 * 1024;

export interface LegacyFx {
  temporal?:EffectDefinition['temporal'];
  sourceProcessing?:EffectDefinition['sourceProcessing'];
  sampling?: EffectDefinition['sampling'];
  grid?: EffectDefinition['grid'];
  label: string;
  group: string;
  passes: number;
  params: EffectParamDefinition[];
  frag: string;
  keepOrig: boolean;
  backdrop: boolean;
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
  if (params.filter((p: any) => p&&referenceParam(p)).length > 4) throw new Error(`${what}: at most four layer inputs`);
  const seen = new Set<string>();
  for (const param of params as EffectParamDefinition[]) {
    if (!param || typeof param !== 'object') throw new Error(`${what}: each param must be an object`);
    if (typeof param.k !== 'string' || !PARAM_KEY.test(param.k)) throw new Error(`${what}: invalid param key "${String(param?.k)}" (expected ${PARAM_KEY})`);
    if (seen.has(param.k)) throw new Error(`${what}: duplicate param key "${param.k}"`);
    if (param.type != null && !['number','toggle','color','layer','path','asset'].includes(param.type)) throw new Error(`${what}: invalid parameter type`);
    if (!validEffectParameterDefault(param)) throw new Error(`${what}: invalid contextual default`);
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
  if (def.backdrop != null && typeof def.backdrop !== 'boolean') throw new Error(`${what}: "backdrop" must be a boolean`);
  if(def.coordinates!=null&&def.coordinates!=='layer')throw new Error(`${what}: invalid effect coordinates`);
  const passes = def.passes ?? 1;
  if (!Number.isInteger(passes) || passes < 1 || passes > MAX_PASSES) throw new Error(`${what}: "passes" must be an integer 1..${MAX_PASSES}`);
  validateParams(def.params, what);
  const generated = def.params.filter(p => referenceParam(p)).flatMap(p => [p.k+'UV',p.k+'FromUV',p.k+'Res',p.k+'Midpoint',p.k+'DomainMin',p.k+'DomainMax',p.k+'Size']);
  if('coordinates' in def&&def.coordinates==='layer'||def.params.some(p=>referenceParam(p)))generated.push('layerUV','ownerFromUV','layerRes');
  if (def.params.some(p => generated.includes(p.k))) throw new Error(`${what}: layer input uniform name collision`);
  validateFrag(def.frag, what);
  const numeric = (key: unknown) => typeof key === 'string' && def.params.some(param => param.k === key && (!param.type || param.type === 'number'));
  if (def.sampling && (!numeric(def.sampling.radius) || !Number.isFinite(def.sampling.support)
      || def.sampling.support <= 0 || def.sampling.support > 8 || !Number.isInteger(def.sampling.maxRadius)
      || def.sampling.maxRadius < 1 || def.sampling.maxRadius > 64
      || def.sampling.space != null && def.sampling.space !== 'layer'
      || def.sampling.firstPass != null && (!Number.isInteger(def.sampling.firstPass) || def.sampling.firstPass < 0 || def.sampling.firstPass >= passes))) throw new Error(`${what}: invalid spatial sampling contract`);
  if(def.temporal&&(!numeric(def.temporal.step)||!numeric(def.temporal.count)||!Number.isInteger(def.temporal.maxSamples)||def.temporal.maxSamples<1||def.temporal.maxSamples>32||passes!==1||def.backdrop||def.keepOrig||def.sampling||def.grid||def.coordinates||def.params.some(referenceParam)||Object.keys(def.sourceProcessing??{}).length>0||!def.rawShader))throw new Error(`${what}: invalid temporal contract`);
  if(def.sourceProcessing){const b=def.sourceProcessing;if(b.geometry&&(!Array.isArray(b.geometry)||b.geometry.length>4||b.geometry.some(k=>!def.params.some(p=>p.k===k&&p.type==='path'&&p.pathBrush))))throw new Error(`${what}: invalid source geometry contract`);if(!Array.isArray(b.padding??[])||(b.padding??[]).some(k=>!numeric(k))||!Number.isFinite(b.factor??1)||(b.factor??1)<0||(b.factor??1)>8||!Number.isFinite(b.pixels??0)||(b.pixels??0)<0||(b.pixels??0)>4096||b.when!=null&&!def.params.some(p=>p.k===b.when&&p.type==='toggle'))throw new Error(`${what}: invalid source bounds contract`);}
  if (def.grid && (passes !== 2 || !numeric(def.grid.columns) || !numeric(def.grid.rows)
      || def.grid.pointSample != null && !def.params.some(param => param.k === def.grid!.pointSample && param.type === 'toggle'))) throw new Error(`${what}: invalid cell grid contract`);
  if (def.viewportPaddingFactor != null && (!Number.isFinite(def.viewportPaddingFactor) || def.viewportPaddingFactor <= 0
      || def.viewportPaddingFactor > 8)) throw new Error(`${what}: invalid viewport padding factor`);

  if (def.viewportPaddingPixels != null && (!Number.isFinite(def.viewportPaddingPixels) || def.viewportPaddingPixels < 0
      || def.viewportPaddingPixels > 4096)) throw new Error(`${what}: invalid fixed viewport padding`);

  for(const p of def.params)if(p.type==='path'&&p.pathWidth&&!numeric(p.pathWidth))throw new Error(`${what}: invalid path width parameter`);
  for(const p of def.params){
    if(p.type==='asset'&&p.assetKind!=null&&!['image','lut'].includes(p.assetKind))throw new Error(`${what}: invalid asset kind`);
    if(p.type==='path'&&p.pathBrush){const b=p.pathBrush;if(!(['hardness','spacing','start','end'] as const).every(k=>numeric(b[k]))||!(['allMasks','sequential'] as const).every(k=>def.params.some(p=>p.k===b[k]&&p.type==='toggle')))throw new Error(`${what}: invalid path brush contract`);}
  }
  const version = effectVersion(def.version);
  if (version === undefined) throw new Error(`${what}: invalid implementation version`);
  if (def.legacy != null) {
    if (!Array.isArray(def.legacy) || def.legacy.length > 32) throw new Error(`${what}: invalid historical revisions`);
    const seen = new Set<number>();
    for (const revision of def.legacy) {
      const older = effectVersion(revision?.version);
      if (!revision || revision.id !== def.id || revision.legacy || revision.version == null
          || older === undefined || older >= version || seen.has(older)) throw new Error(`${what}: invalid historical revision`);
      seen.add(older); validateEffect(revision);
    }
  }
  if (def.actions != null) {
    if (!Array.isArray(def.actions) || def.actions.length > 16) throw new Error(`${what}: invalid parameter actions`);
    for (const action of def.actions) {
      if (action?.kind !== 'swap' || typeof action.label !== 'string' || !action.label
          || !Array.isArray(action.keys) || action.keys.length !== 2 || action.keys[0] === action.keys[1]
          || action.keys.some(key => def.params.find(param => param.k === key)?.type !== 'color')) throw new Error(`${what}: invalid color swap action`);
    }
  }
  return def;
}

export function validateTransition(def: TransitionDefinition): TransitionDefinition {
  if (!def || typeof def !== 'object') throw new Error('transition: definition must be an object');
  const what = `transition "${String(def.id)}"`;
  if (def.params?.some(p => referenceParam(p))) throw new Error(`${what}: layer inputs require an effect`);
  if (typeof def.id !== 'string' || !EFFECT_ID.test(def.id)) throw new Error(`transition: invalid id "${String(def.id)}"`);
  if (typeof def.label !== 'string' || !def.label) throw new Error(`${what}: "label" is required`);
  validateParams(def.params, what);
  const generated = def.params.filter(p => referenceParam(p)).flatMap(p => [p.k+'UV',p.k+'FromUV',p.k+'Res',p.k+'Midpoint',p.k+'DomainMin',p.k+'DomainMax',p.k+'Size']);
  if('coordinates' in def&&def.coordinates==='layer'||def.params.some(p=>referenceParam(p)))generated.push('layerUV','ownerFromUV','layerRes');
  if (def.params.some(p => generated.includes(p.k))) throw new Error(`${what}: layer input uniform name collision`);
  validateFrag(def.frag, what);
  return def;
}

/** `uniform` declarations for the named params, in declaration order. */
export function paramDeclarations(params: EffectParamDefinition[]): string[] {
  return params.flatMap(param => referenceParam(param) ? [
    ...(param.type==='asset'&&param.assetKind==='lut'?[`uniform vec3 u_${param.k}DomainMin,u_${param.k}DomainMax; uniform float u_${param.k}Size;`]:[]),
    `uniform sampler2D u_${param.k}; uniform float u_has_${param.k}; uniform float u_${param.k}Midpoint; uniform mat3 u_${param.k}UV; uniform mat3 u_${param.k}FromUV; uniform vec2 u_${param.k}Res;`,
    `vec4 sample_${param.k}(vec2 q){ if(u_has_${param.k}<.5||any(lessThan(q,vec2(0.)))||any(greaterThan(q,vec2(1.))))return vec4(0.); vec3 px=u_${param.k}FromUV*vec3(q,1.); vec2 st=px.xy/px.z/u_res; st.y=1.-st.y; if(any(lessThan(st,vec2(0.)))||any(greaterThan(st,vec2(1.))))return vec4(0.); return texture(u_${param.k},st); }`
  ] : [`uniform ${isColor(param) ? 'vec3' : 'float'} u_${param.k};`]);
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
  return params.filter(param => !referenceParam(param)).map((param) => { const index = params.indexOf(param); return `#define ${isColor(param) ? 'u_c' : 'u_p'}${index} u_${param.k}`; });
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
  const backdrop = def.backdrop === true;
  const frag = def.rawShader
    ? def.frag
    : assemble(
        pre,
        [
          ...(('coordinates' in def&&def.coordinates==='layer'||params.some(p => referenceParam(p))) ? ['uniform mat3 u_layerUV; uniform mat3 u_ownerFromUV; uniform vec2 u_layerRes;'] : []),
          ...paramDeclarations(params),
          ...unlessDeclared(pre, 'pass', 'uniform int u_pass;'),
          ...(keepOrig ? unlessDeclared(pre, 'orig', 'uniform sampler2D u_orig;') : []),
          ...(backdrop ? unlessDeclared(pre, 'backdrop', 'uniform sampler2D u_backdrop;') : []),
          ...paramAliases(params)
        ],
        def.frag
      );
  return { label: def.label, group: def.group, passes, params, frag, keepOrig, backdrop, sampling: def.sampling, grid: def.grid, sourceProcessing:def.sourceProcessing, temporal:def.temporal };
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
          ...(('coordinates' in def&&def.coordinates==='layer'||params.some(p => referenceParam(p))) ? ['uniform mat3 u_layerUV; uniform mat3 u_ownerFromUV; uniform vec2 u_layerRes;'] : []),
          ...paramDeclarations(params),
          ...paramAliases(params)
        ],
        def.frag
      );
  return { label: def.label, group: def.group ?? 'Transitions', params, frag };
}
