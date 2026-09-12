import type { EffectParamDefinition, ExtensionLayerDefinition } from './api';

const ID = /^[a-z][a-z0-9._-]{0,79}$/;
const PARAM = /^[a-zA-Z][a-zA-Z0-9_]{0,63}$/;
const COLOR = /^#[0-9a-f]{6}$/i;
const MAX_FRAGMENT_BYTES = 96_000;
const MAX_PARAMS = 48;
const FORBIDDEN_KEYS = new Set(['__proto__', 'prototype', 'constructor']);

function validateJson(value: unknown, path: string, seen: WeakSet<object>, budget: { nodes: number }, depth = 0): void {
  if (++budget.nodes > 10_000 || depth > 32) throw new Error(`extension layer defaults are too large at ${path}`);
  if (value == null || typeof value === 'string' || typeof value === 'boolean') return;
  if (typeof value === 'number' && Number.isFinite(value)) return;
  if (!value || typeof value !== 'object') throw new Error(`extension layer defaults must be JSON at ${path}`);
  if (seen.has(value)) throw new Error(`extension layer defaults contain a cycle at ${path}`);
  seen.add(value);
  if (Array.isArray(value)) {
    value.forEach((item, index) => validateJson(item, `${path}[${index}]`, seen, budget, depth + 1));
    return;
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) throw new Error(`extension layer defaults must use plain objects at ${path}`);
  for (const [key, item] of Object.entries(value)) {
    if (FORBIDDEN_KEYS.has(key)) throw new Error(`extension layer defaults contain unsafe key "${key}"`);
    validateJson(item, `${path}.${key}`, seen, budget, depth + 1);
  }
}

function validateParam(param: EffectParamDefinition, seen: Set<string>): void {
  if (!param || typeof param !== 'object') throw new Error('extension layer parameter must be an object');
  if (!PARAM.test(param.k)) throw new Error(`extension layer parameter has invalid key "${String(param.k)}"`);
  if (seen.has(param.k)) throw new Error(`extension layer parameter "${param.k}" is duplicated`);
  seen.add(param.k);
  if (typeof param.label !== 'string' || !param.label.trim()) throw new Error(`extension layer parameter "${param.k}" needs a label`);
  const raw = param as EffectParamDefinition & { type?: string };
  const key = param.k;
  if (raw.type === 'color') {
    if (typeof raw.def !== 'string' || !COLOR.test(raw.def)) throw new Error(`extension layer parameter "${key}" needs a hex color default`);
    return;
  }
  if (raw.type === 'toggle') {
    if (typeof raw.def !== 'boolean') throw new Error(`extension layer parameter "${key}" needs a boolean default`);
    return;
  }
  if (![raw.def, (raw as any).min, (raw as any).max].every((value) => typeof value === 'number' && Number.isFinite(value))) {
    throw new Error(`extension layer parameter "${key}" needs finite numeric bounds`);
  }
  const numeric = raw as { def: number; min: number; max: number };
  if (numeric.min > numeric.max || numeric.def < numeric.min || numeric.def > numeric.max) {
    throw new Error(`extension layer parameter "${key}" default must be inside its bounds`);
  }
}

export function validateExtensionLayerDefinition(definition: ExtensionLayerDefinition): void {
  if (!definition || typeof definition !== 'object') throw new Error('layers.register requires a definition');
  if (!ID.test(definition.id)) throw new Error(`extension layer has invalid id "${String(definition.id)}"`);
  if (typeof definition.label !== 'string' || !definition.label.trim()) throw new Error(`extension layer "${definition.id}" needs a label`);
  if (!Number.isSafeInteger(definition.version) || definition.version < 1) throw new Error(`extension layer "${definition.id}" needs a positive integer version`);
  if (definition.color != null && !COLOR.test(definition.color)) throw new Error(`extension layer "${definition.id}" has an invalid color`);
  for (const [key, value] of [['width', definition.width], ['height', definition.height]] as const) {
    if (value != null && (!Number.isFinite(value) || value < 1 || value > 16384)) {
      throw new Error(`extension layer "${definition.id}" ${key} must be between 1 and 16384`);
    }
  }
  if (!Array.isArray(definition.params) || definition.params.length > MAX_PARAMS) {
    throw new Error(`extension layer "${definition.id}" supports at most ${MAX_PARAMS} parameters`);
  }
  const seen = new Set<string>();
  for (const param of definition.params) validateParam(param, seen);
  if (definition.defaults != null) validateJson(definition.defaults, 'defaults', new WeakSet(), { nodes: 0 });
  if (definition.renderer?.kind === 'mesh') {
    if (!PARAM.test(definition.renderer.assetField)) {
      throw new Error(`extension layer "${definition.id}" mesh renderer needs a safe assetField`);
    }
  } else if (definition.renderer?.kind === 'fragment') {
    const fragment = definition.renderer.fragment;
    if (typeof fragment !== 'string' || !/\bvoid\s+main\s*\(/.test(fragment)) {
      throw new Error(`extension layer "${definition.id}" fragment needs void main()`);
    }
    if (new TextEncoder().encode(fragment).length > MAX_FRAGMENT_BYTES) {
      throw new Error(`extension layer "${definition.id}" fragment is too large`);
    }
  } else {
    throw new Error(`extension layer "${definition.id}" needs a supported renderer`);
  }
}

/** Assemble the renderer without persisting its code in the project file. */
export function extensionLayerFragment(definition: ExtensionLayerDefinition, preamble: string): string {
  if (definition.renderer.kind !== 'fragment') throw new Error(`extension layer "${definition.id}" is not a fragment renderer`);
  const uniforms = definition.params.map((param) => {
    if (param.type === 'color') return `uniform vec3 u_${param.k};`;
    if (param.type === 'toggle') return `uniform bool u_${param.k};`;
    return `uniform float u_${param.k};`;
  }).join('\n');
  return `${preamble}\n${uniforms}\n${definition.renderer.fragment}`;
}
