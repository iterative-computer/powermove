import type { BlendMode, Layer, LayerType } from '../types/project';
import type {
  Transform,
  TransformAggregate,
  TransformEdit,
  TransformExpression,
  OneToEightTransformArgs,
  TwoToEightTransformArgs,
  TransformOp,
  TransformOperationExpression,
  TransformOrder,
  TransformPath,
  TransformPrimitive,
  TransformRef,
  TransformScope,
  TransformStateOrder
} from '../types/transform';
import {
  COMMAND_TYPES,
  type AddEffectCommand,
  type AddLayerCommand,
  type AddMarkerCommand,
  type CommandKeyframe,
  type CompositionPatch,
  type CreateSectionCommand,
  type DeleteLayersCommand,
  type EditCommand,
  type Easing,
  type JsonObject,
  type JsonValue,
  type LayerPatch,
  type LayerTarget,
  type ReplaceKeyframesCommand,
  type SectionManifest,
  type SectionVersion,
  type SetCompositionCommand,
  type SetContentCommand,
  type SetEasingCommand,
  type SetEffectCommand,
  type SetExpressionCommand,
  type SetLayerCommand,
  type SetPropertyCommand,
  type SetSceneParameterCommand,
  type SetTransitionCommand,
  type TransformLayersCommand,
  type UpdateSectionCommand
} from '../types/commands';

export const COMMAND_JSON_LIMIT = 50_000;
export const MAX_COMMANDS = 80;
export const MAX_KEYFRAMES = 80;
export const MAX_EASING_TARGETS = 1_000;
export const MAX_DELETE_TARGETS = 20;
export const MAX_EXPRESSION_CHARS = 2_000;

const FORBIDDEN_KEYS = new Set(['__proto__', 'prototype', 'constructor']);
const COMMAND_TYPE_SET = new Set<string>(COMMAND_TYPES);
const LAYER_TYPES = new Set<LayerType>([
  'solid', 'text', 'shape', 'image', 'video', 'audio', 'adjustment', 'shader', 'extension', 'null', 'precomp'
]);
const BLEND_MODES = new Set([
  'normal', 'add', 'screen', 'multiply', 'overlay', 'softlight', 'difference', 'lighten', 'darken'
]);
const LAYER_FIELDS = new Set([
  'name', 'from', 'duration', 'visible', 'locked', 'shy', 'blend',
  'motionBlur', 'parent', 'color', 'collapsed', 'scaleLinked'
]);
const COMPOSITION_FIELDS = new Set([
  'name', 'width', 'height', 'fps', 'duration', 'background', 'backgroundFill',
  'shutter', 'workArea'
]);
const EFFECT_FIELDS = new Set(['enabled', 'open']);
const EASING_PRESETS = new Set([
  'linear', 'ease', 'easeIn', 'easeOut', 'easeInOut', 'quadIn', 'quadOut', 'quadInOut',
  'cubicIn', 'cubicOut', 'cubicInOut', 'quartIn', 'quartOut', 'quartInOut', 'expoIn',
  'expoOut', 'expoInOut', 'circIn', 'circOut', 'circInOut', 'backIn', 'backOut',
  'backInOut', 'power', 'snap', 'glide'
]);

const COMMAND_FIELDS: Record<(typeof COMMAND_TYPES)[number], readonly string[]> = {
  set_property: [
    'type', 'target', 'path', 'value', 'time', 'mode', 'ease', 'hold',
    'overrideLock', 'preserveHandEdits', 'markIntent'
  ],
  replace_keyframes: [
    'type', 'target', 'path', 'keyframes', 'replace', 'expression',
    'overrideLock', 'preserveHandEdits'
  ],
  set_easing: ['type', 'keyframes', 'curve'],
  set_expression: ['type', 'target', 'path', 'expression'],
  set_content: ['type', 'target', 'patch', 'overrideLock'],
  set_layer: ['type', 'target', 'patch'],
  set_composition: ['type', 'patch'],
  add_layer: [
    'type', 'id', 'layerType', 'name', 'from', 'duration', 'content', 'properties',
    'color', 'index', 'select', 'parent', 'blend', 'motionBlur', 'visible',
    'shy', 'collapsed'
  ],
  delete_layers: ['type', 'target', 'targets'],
  reorder_layer: ['type', 'target', 'index'],
  add_effect: ['type', 'target', 'effect', 'parameters', 'open', 'enabled'],
  remove_effect: ['type', 'target', 'effect'],
  set_effect: ['type', 'target', 'effect', 'patch'],
  set_transition: ['type', 'layer', 'edge', 'transition'],
  set_scene_parameter: ['type', 'name', 'label', 'control', 'value', 'min', 'max', 'options'],
  add_marker: ['type', 'id', 'time', 'name'],
  create_section: ['type', 'section'],
  update_section: ['type', 'sectionId', 'layers', 'thumb', 'at', 'version'],
  transform_layers: ['type', 'transform', 'state']
};

const AGENT_COMMAND_FIELDS = {
  set_property: ['type', 'target', 'path', 'value', 'time', 'mode', 'ease', 'hold'],
  replace_keyframes: ['type', 'target', 'path', 'keyframes', 'replace', 'expression'],
  set_expression: COMMAND_FIELDS.set_expression,
  set_content: ['type', 'target', 'patch'],
  set_layer: COMMAND_FIELDS.set_layer,
  set_composition: COMMAND_FIELDS.set_composition,
  add_layer: COMMAND_FIELDS.add_layer,
  delete_layers: COMMAND_FIELDS.delete_layers,
  reorder_layer: COMMAND_FIELDS.reorder_layer,
  add_effect: COMMAND_FIELDS.add_effect,
  remove_effect: COMMAND_FIELDS.remove_effect,
  set_effect: COMMAND_FIELDS.set_effect,
  set_transition: COMMAND_FIELDS.set_transition,
  set_scene_parameter: COMMAND_FIELDS.set_scene_parameter,
  add_marker: COMMAND_FIELDS.add_marker,
  transform_layers: COMMAND_FIELDS.transform_layers
} as const;
const AGENT_COMMAND_TYPE_SET = new Set<string>(Object.keys(AGENT_COMMAND_FIELDS));

const SCOPES = new Set<TransformScope>(['selection', 'all', 'visible']);
const ORDERS = new Set<TransformOrder>([
  'stack', 'reverseStack', 'selection', 'reverseSelection', 'start', 'reverseStart',
  'name', 'random'
]);
const OPS = new Set<TransformOp>([
  'add', 'subtract', 'multiply', 'divide', 'min', 'max', 'clamp', 'round',
  'floor', 'ceil', 'abs', 'negate', 'frames', 'equal', 'if'
]);
const OP_ARITY: Record<string, readonly [number, number]> = {
  add: [2, 8], subtract: [2, 2], multiply: [2, 8], divide: [2, 2],
  min: [1, 8], max: [1, 8], clamp: [3, 3], round: [1, 1], floor: [1, 1],
  ceil: [1, 1], abs: [1, 1], negate: [1, 1], frames: [1, 1], equal: [2, 2], if: [3, 3]
};
const REFS = new Set<TransformRef>(['current', 'index', 'count', 'fps', 'playhead', 'composition.duration']);
const AGGREGATES = new Set<TransformAggregate>(['min', 'max', 'first', 'last', 'sum', 'average']);
const SAFE_STATE_KEY = /^[a-z][a-z0-9_.-]{0,79}$/i;
const LAYER_PATHS = new Set([
  'layer.name', 'layer.from', 'layer.duration', 'layer.visible', 'layer.locked',
  'layer.shy', 'layer.blend', 'layer.motionBlur', 'layer.parent',
  'layer.color', 'layer.collapsed'
]);

export class ValidationError extends Error {
  readonly path?: string;

  constructor(message: string, path?: string) {
    super(path ? `${path}: ${message}` : message);
    this.name = 'ValidationError';
    this.path = path;
  }
}

const invalid = (message: string, path?: string): ValidationError => new ValidationError(message, path);
const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const owns = (value: Record<string, unknown>, key: string): boolean => Object.hasOwn(value, key);

function finite(value: unknown, path: string): number | ValidationError {
  let number: number;
  try {
    number = Number(value);
  } catch {
    return invalid('must be a finite number', path);
  }
  return Number.isFinite(number) ? number : invalid('must be a finite number', path);
}

function requiredString(value: unknown, path: string): string | ValidationError {
  return typeof value === 'string' && value.length > 0 ? value : invalid('must be a non-empty string', path);
}

function stringified(value: unknown): string {
  return String(value);
}

function target(value: unknown, path: string): LayerTarget | ValidationError {
  if (value === null || typeof value === 'string') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  return invalid('must be null, a selection token, a layer index, ID, or name', path);
}

function cloneJson(value: unknown, path: string): JsonValue | ValidationError {
  try {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) return invalid('must be JSON-serialisable', path);
    return JSON.parse(encoded) as JsonValue;
  } catch {
    return invalid('must be JSON-serialisable', path);
  }
}

function safePatch(value: unknown, path: string): JsonObject | ValidationError {
  if (!isRecord(value)) return invalid('must be an object', path);
  for (const key of Object.keys(value)) {
    if (FORBIDDEN_KEYS.has(key)) return invalid(`contains unsafe key "${key}"`, path);
  }
  const cloned = cloneJson(value, path);
  return cloned instanceof ValidationError ? cloned : cloned as JsonObject;
}

function optionalTarget(source: Record<string, unknown>, out: { target?: LayerTarget }): ValidationError | null {
  if (!owns(source, 'target') || source.target === undefined) return null;
  const parsed = target(source.target, 'target');
  if (parsed instanceof ValidationError) return parsed;
  out.target = parsed;
  return null;
}

function optionalBoolean<K extends string>(
  source: Record<string, unknown>,
  out: Partial<Record<K, boolean>>,
  key: K
): ValidationError | null {
  if (!owns(source, key) || source[key] === undefined) return null;
  if (typeof source[key] !== 'boolean') return invalid('must be a boolean', key);
  out[key] = source[key];
  return null;
}

function checkedSize(command: EditCommand): EditCommand | ValidationError {
  try {
    return JSON.stringify(command).length <= COMMAND_JSON_LIMIT
      ? command
      : invalid(`command exceeds ${COMMAND_JSON_LIMIT} JSON characters`);
  } catch {
    return invalid('command must be JSON-serialisable');
  }
}

function preflightSelectedSize(source: Record<string, unknown>): ValidationError | null {
  if (typeof source.type !== 'string' || !COMMAND_TYPE_SET.has(source.type)) return null;
  const selected: Record<string, unknown> = {};
  for (const key of COMMAND_FIELDS[source.type as keyof typeof COMMAND_FIELDS]) {
    if (source[key] !== undefined) selected[key] = source[key];
  }
  try {
    return JSON.stringify(selected).length <= COMMAND_JSON_LIMIT
      ? null
      : invalid(`command exceeds ${COMMAND_JSON_LIMIT} JSON characters`);
  } catch {
    return invalid('command must be JSON-serialisable');
  }
}

function parseSetProperty(source: Record<string, unknown>): SetPropertyCommand | ValidationError {
  const path = requiredString(source.path, 'path');
  if (path instanceof ValidationError) return path;
  if (!owns(source, 'value')) return invalid('is required', 'value');
  const value = cloneJson(source.value, 'value');
  if (value instanceof ValidationError) return value;
  const out: SetPropertyCommand = { type: 'set_property', path, value };
  const targetError = optionalTarget(source, out);
  if (targetError) return targetError;
  if (owns(source, 'time') && source.time != null) {
    const time = finite(source.time, 'time');
    if (time instanceof ValidationError) return time;
    out.time = time;
  }
  if (owns(source, 'mode') && source.mode != null) {
    if (!['auto', 'static', 'keyframe'].includes(String(source.mode))) return invalid('is invalid', 'mode');
    out.mode = String(source.mode) as SetPropertyCommand['mode'];
  }
  if (owns(source, 'ease') && source.ease != null) out.ease = stringified(source.ease);
  if (owns(source, 'hold') && source.hold != null) out.hold = Boolean(source.hold);
  for (const key of ['overrideLock', 'preserveHandEdits'] as const) {
    const fieldError = optionalBoolean(source, out, key);
    if (fieldError) return fieldError;
  }
  if (owns(source, 'markIntent') && source.markIntent !== undefined) {
    if (!(typeof source.markIntent === 'boolean' || typeof source.markIntent === 'string')) {
      return invalid('must be a boolean or string', 'markIntent');
    }
    out.markIntent = source.markIntent;
  }
  return out;
}

function parseReplaceKeyframes(source: Record<string, unknown>): ReplaceKeyframesCommand | ValidationError {
  const path = requiredString(source.path, 'path');
  if (path instanceof ValidationError) return path;
  if (!Array.isArray(source.keyframes)) return invalid('must be an array', 'keyframes');
  const keyframes: CommandKeyframe[] = [];
  for (const [index, item] of source.keyframes.slice(0, MAX_KEYFRAMES).entries()) {
    if (!isRecord(item)) return invalid('must be an object', `keyframes[${index}]`);
    const time = finite(item.time, `keyframes[${index}].time`);
    if (time instanceof ValidationError) return time;
    if (!owns(item, 'value')) return invalid('is required', `keyframes[${index}].value`);
    const value = cloneJson(item.value, `keyframes[${index}].value`);
    if (value instanceof ValidationError) return value;
    const keyframe: CommandKeyframe = { time: Math.max(0, time), value };
    if (owns(item, 'ease') && item.ease != null) keyframe.ease = stringified(item.ease);
    if (owns(item, 'hold')) keyframe.hold = Boolean(item.hold);
    keyframes.push(keyframe);
  }
  const out: ReplaceKeyframesCommand = { type: 'replace_keyframes', path, keyframes };
  const targetError = optionalTarget(source, out);
  if (targetError) return targetError;
  if (owns(source, 'replace') && source.replace != null) out.replace = Boolean(source.replace);
  if (owns(source, 'expression')) {
    if (!(source.expression == null || typeof source.expression === 'string')) {
      return invalid('must be a string or null', 'expression');
    }
    if (typeof source.expression === 'string' && source.expression.length > MAX_EXPRESSION_CHARS) {
      return invalid(`must be ${MAX_EXPRESSION_CHARS} characters or fewer`, 'expression');
    }
    out.expression = source.expression;
  }
  for (const key of ['overrideLock', 'preserveHandEdits'] as const) {
    const fieldError = optionalBoolean(source, out, key);
    if (fieldError) return fieldError;
  }
  return out;
}

function easing(value: unknown): Easing | ValidationError {
  if (typeof value === 'string') {
    return EASING_PRESETS.has(value) ? value : invalid('is not a known easing preset', 'curve');
  }
  if (!Array.isArray(value) || value.length !== 4) return invalid('must be a preset or four handles', 'curve');
  const handles: number[] = [];
  for (const [index, part] of value.entries()) {
    const parsed = finite(part, `curve[${index}]`);
    if (parsed instanceof ValidationError) return parsed;
    handles.push(parsed);
  }
  return [
    Math.max(0, Math.min(1, handles[0]!)),
    Math.max(-4, Math.min(4, handles[1]!)),
    Math.max(0, Math.min(1, handles[2]!)),
    Math.max(-4, Math.min(4, handles[3]!))
  ];
}

function parseSetEasing(source: Record<string, unknown>): SetEasingCommand | ValidationError {
  if (!Array.isArray(source.keyframes)) return invalid('must be an array', 'keyframes');
  const keyframes = source.keyframes
    .filter((id): id is string => typeof id === 'string' && id.length > 0)
    .slice(0, MAX_EASING_TARGETS);
  if (!keyframes.length) return invalid('must contain at least one keyframe ID', 'keyframes');
  const curve = easing(source.curve);
  return curve instanceof ValidationError ? curve : { type: 'set_easing', keyframes: [...new Set(keyframes)], curve };
}

function parseSetExpression(source: Record<string, unknown>): SetExpressionCommand | ValidationError {
  const path = requiredString(source.path, 'path');
  if (path instanceof ValidationError) return path;
  if (owns(source, 'expression') && !(source.expression == null || typeof source.expression === 'string')) {
    return invalid('must be a string or null', 'expression');
  }
  if (typeof source.expression === 'string' && source.expression.length > MAX_EXPRESSION_CHARS) {
    return invalid(`must be ${MAX_EXPRESSION_CHARS} characters or fewer`, 'expression');
  }
  const expression = typeof source.expression === 'string' ? source.expression : null;
  const out: SetExpressionCommand = { type: 'set_expression', path, expression };
  return optionalTarget(source, out) ?? out;
}

function parseSetContent(source: Record<string, unknown>): SetContentCommand | ValidationError {
  const patch = safePatch(source.patch, 'patch');
  if (patch instanceof ValidationError) return patch;
  const out: SetContentCommand = { type: 'set_content', patch };
  const targetError = optionalTarget(source, out);
  if (targetError) return targetError;
  const lockError = optionalBoolean(source, out, 'overrideLock');
  return lockError ?? out;
}

function parseLayerPatch(value: unknown): LayerPatch | ValidationError {
  const patch = safePatch(value, 'patch');
  if (patch instanceof ValidationError) return patch;
  for (const key of Object.keys(patch)) if (!LAYER_FIELDS.has(key)) return invalid(`unknown layer field "${key}"`, 'patch');
  const out: LayerPatch = {};
  for (const key of ['from', 'duration'] as const) {
    if (patch[key] == null) continue;
    const number = finite(patch[key], `patch.${key}`);
    if (number instanceof ValidationError) return number;
    out[key] = number;
  }
  for (const key of ['visible', 'locked', 'shy', 'motionBlur', 'collapsed', 'scaleLinked'] as const) {
    if (patch[key] != null) out[key] = Boolean(patch[key]);
  }
  for (const key of ['name', 'color'] as const) if (patch[key] != null) out[key] = stringified(patch[key]);
  if (patch.blend != null) {
    const blend = stringified(patch.blend);
    if (!BLEND_MODES.has(blend)) return invalid('is not a known blend mode', 'patch.blend');
    out.blend = blend as BlendMode;
  }
  if (owns(patch, 'parent')) {
    const parent = target(patch.parent, 'patch.parent');
    if (parent instanceof ValidationError) return parent;
    out.parent = parent;
  }
  return out;
}

function parseSetLayer(source: Record<string, unknown>): SetLayerCommand | ValidationError {
  const patch = parseLayerPatch(source.patch);
  if (patch instanceof ValidationError) return patch;
  const out: SetLayerCommand = { type: 'set_layer', patch };
  return optionalTarget(source, out) ?? out;
}

function parseCompositionPatch(value: unknown): CompositionPatch | ValidationError {
  const patch = safePatch(value, 'patch');
  if (patch instanceof ValidationError) return patch;
  for (const key of Object.keys(patch)) {
    if (!COMPOSITION_FIELDS.has(key)) return invalid(`unknown composition field "${key}"`, 'patch');
  }
  const out: CompositionPatch = {};
  for (const key of ['width', 'height', 'fps', 'duration', 'shutter'] as const) {
    if (patch[key] == null) continue;
    const number = finite(patch[key], `patch.${key}`);
    if (number instanceof ValidationError) return number;
    out[key] = number;
  }
  for (const key of ['name', 'background'] as const) if (patch[key] != null) out[key] = stringified(patch[key]);
  if (patch.backgroundFill != null) out.backgroundFill = patch.backgroundFill;
  if (patch.workArea != null) {
    if (!Array.isArray(patch.workArea) || patch.workArea.length !== 2) {
      return invalid('must contain start and end', 'patch.workArea');
    }
    const start = finite(patch.workArea[0], 'patch.workArea[0]');
    if (start instanceof ValidationError) return start;
    const end = finite(patch.workArea[1], 'patch.workArea[1]');
    if (end instanceof ValidationError) return end;
    out.workArea = [start, end];
  }
  return out;
}

function parseSetComposition(source: Record<string, unknown>): SetCompositionCommand | ValidationError {
  const patch = parseCompositionPatch(source.patch);
  return patch instanceof ValidationError ? patch : { type: 'set_composition', patch };
}

function parseAddLayer(source: Record<string, unknown>): AddLayerCommand | ValidationError {
  if (typeof source.layerType !== 'string' || !LAYER_TYPES.has(source.layerType as LayerType)) {
    return invalid('is not a known layer type', 'layerType');
  }
  const out: AddLayerCommand = { type: 'add_layer', layerType: source.layerType as LayerType };
  for (const key of ['id', 'name', 'color'] as const) if (source[key] != null) out[key] = stringified(source[key]);
  for (const key of ['from', 'duration', 'index'] as const) {
    if (source[key] == null) continue;
    const number = finite(source[key], key);
    if (number instanceof ValidationError) return number;
    out[key] = number;
  }
  for (const key of ['select', 'motionBlur', 'visible', 'shy', 'collapsed'] as const) {
    if (source[key] != null) out[key] = Boolean(source[key]);
  }
  for (const key of ['content', 'properties'] as const) {
    if (source[key] === undefined) continue;
    const patch = safePatch(source[key], key);
    if (patch instanceof ValidationError) return patch;
    out[key] = patch;
  }
  if (owns(source, 'parent') && source.parent !== undefined) {
    const parent = target(source.parent, 'parent');
    if (parent instanceof ValidationError) return parent;
    out.parent = parent;
  }
  if (source.blend != null) {
    const blend = stringified(source.blend);
    if (!BLEND_MODES.has(blend)) return invalid('is not a known blend mode', 'blend');
    out.blend = blend as BlendMode;
  }
  if (out.layerType === 'audio' && (out.parent != null || (out.blend != null && out.blend !== 'normal') || out.motionBlur === true)) {
    return invalid('audio layers do not support parenting, non-normal blending, or motion blur');
  }
  return out;
}

function parseDeleteLayers(source: Record<string, unknown>): DeleteLayersCommand | ValidationError {
  const out: DeleteLayersCommand = { type: 'delete_layers' };
  const targetError = optionalTarget(source, out);
  if (targetError) return targetError;
  if (owns(source, 'targets')) {
    if (!Array.isArray(source.targets)) return invalid('must be an array', 'targets');
    const targets: LayerTarget[] = [];
    for (const [index, item] of source.targets.slice(0, MAX_DELETE_TARGETS).entries()) {
      const parsed = target(item, `targets[${index}]`);
      if (parsed instanceof ValidationError) return parsed;
      targets.push(parsed);
    }
    out.targets = targets;
  }
  return out;
}

function parseReorderLayer(source: Record<string, unknown>): EditCommand | ValidationError {
  const index = finite(source.index, 'index');
  if (index instanceof ValidationError) return index;
  const out: EditCommand = { type: 'reorder_layer', index };
  return optionalTarget(source, out) ?? out;
}

function effectName(value: unknown): string | ValidationError {
  return requiredString(value, 'effect');
}

function parseAddEffect(source: Record<string, unknown>): AddEffectCommand | ValidationError {
  const effect = effectName(source.effect);
  if (effect instanceof ValidationError) return effect;
  const out: AddEffectCommand = { type: 'add_effect', effect };
  const targetError = optionalTarget(source, out);
  if (targetError) return targetError;
  if (source.parameters !== undefined) {
    const parameters = safePatch(source.parameters, 'parameters');
    if (parameters instanceof ValidationError) return parameters;
    out.parameters = parameters;
  }
  if (source.open != null) out.open = Boolean(source.open);
  if (source.enabled != null) out.enabled = Boolean(source.enabled);
  return out;
}

function parseRemoveEffect(source: Record<string, unknown>): EditCommand | ValidationError {
  const effect = effectName(source.effect);
  if (effect instanceof ValidationError) return effect;
  const out: EditCommand = { type: 'remove_effect', effect };
  return optionalTarget(source, out) ?? out;
}

function parseSetEffect(source: Record<string, unknown>): SetEffectCommand | ValidationError {
  const effect = effectName(source.effect);
  if (effect instanceof ValidationError) return effect;
  const patch = safePatch(source.patch, 'patch');
  if (patch instanceof ValidationError) return patch;
  for (const key of Object.keys(patch)) if (!EFFECT_FIELDS.has(key)) return invalid(`unknown effect field "${key}"`, 'patch');
  const out: SetEffectCommand = { type: 'set_effect', effect, patch: {} };
  const targetError = optionalTarget(source, out);
  if (targetError) return targetError;
  if (patch.enabled != null) out.patch.enabled = Boolean(patch.enabled);
  if (patch.open != null) out.patch.open = Boolean(patch.open);
  return out;
}

function parseSetTransition(source: Record<string, unknown>): SetTransitionCommand | ValidationError {
  const layer = target(source.layer, 'layer');
  if (layer instanceof ValidationError) return layer;
  if (source.edge !== 'in' && source.edge !== 'out') return invalid('must be "in" or "out"', 'edge');
  if (source.transition === null) {
    return { type: 'set_transition', layer, edge: source.edge, transition: null };
  }
  if (!isRecord(source.transition)) return invalid('must be an object or null', 'transition');
  const transitionType = requiredString(source.transition.type, 'transition.type');
  if (transitionType instanceof ValidationError) return transitionType;
  const transition: NonNullable<SetTransitionCommand['transition']> = { type: transitionType };
  if (source.transition.dur !== undefined) {
    const dur = finite(source.transition.dur, 'transition.dur');
    if (dur instanceof ValidationError) return dur;
    if (dur < 0.02 || dur > 600) return invalid('must be between 0.02 and 600', 'transition.dur');
    transition.dur = dur;
  }
  if (source.transition.p !== undefined) {
    const parameters = safePatch(source.transition.p, 'transition.p');
    if (parameters instanceof ValidationError) return parameters;
    const clean: Record<string, number | string | boolean> = {};
    for (const [name, value] of Object.entries(parameters)) {
      if (typeof value === 'number' && Number.isFinite(value)
          || typeof value === 'boolean'
          || typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value)) {
        clean[name] = value;
      } else {
        return invalid('must be a finite number, hex color, or boolean', `transition.p.${name}`);
      }
    }
    transition.p = clean;
  }
  return { type: 'set_transition', layer, edge: source.edge, transition };
}

function parseSetSceneParameter(source: Record<string, unknown>): SetSceneParameterCommand | ValidationError {
  const name = typeof source.name === 'string' ? source.name.trim() : stringified(source.name ?? '').trim();
  if (!name) return invalid('is required', 'name');
  if (!owns(source, 'value')) return invalid('is required', 'value');
  const value = cloneJson(source.value, 'value');
  if (value instanceof ValidationError) return value;
  const out: SetSceneParameterCommand = { type: 'set_scene_parameter', name, value };
  for (const key of ['label', 'control'] as const) if (source[key] != null) out[key] = stringified(source[key]);
  for (const key of ['min', 'max'] as const) {
    if (source[key] == null) continue;
    const number = finite(source[key], key);
    if (number instanceof ValidationError) return number;
    out[key] = number;
  }
  if (source.options != null) {
    if (!Array.isArray(source.options)) return invalid('must be an array', 'options');
    const options = cloneJson(source.options, 'options');
    if (options instanceof ValidationError || !Array.isArray(options)) return options instanceof ValidationError ? options : invalid('must be an array', 'options');
    out.options = options;
  }
  return out;
}

function parseAddMarker(source: Record<string, unknown>): AddMarkerCommand | ValidationError {
  const out: AddMarkerCommand = { type: 'add_marker' };
  if (source.id != null) out.id = stringified(source.id);
  if (source.name != null) out.name = stringified(source.name);
  if (source.time != null) {
    const time = finite(source.time, 'time');
    if (time instanceof ValidationError) return time;
    out.time = time;
  }
  return out;
}

function jsonRecord(value: unknown, path: string): JsonObject | ValidationError {
  if (!isRecord(value)) return invalid('must be an object', path);
  const clone = cloneJson(value, path);
  return clone instanceof ValidationError ? clone : clone as JsonObject;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isChannel(value: unknown): boolean {
  if (!isRecord(value) || !Array.isArray(value.kf)) return false;
  if (!(typeof value.v === 'string' || typeof value.v === 'boolean' || isFiniteNumber(value.v))) return false;
  if (!(value.expr === null || typeof value.expr === 'string')) return false;
  return value.kf.every(keyframe => isRecord(keyframe)
    && isFiniteNumber(keyframe.t)
    && (typeof keyframe.v === 'string' || typeof keyframe.v === 'boolean' || isFiniteNumber(keyframe.v))
    && Array.isArray(keyframe.eo) && keyframe.eo.length === 2 && keyframe.eo.every(isFiniteNumber)
    && Array.isArray(keyframe.ei) && keyframe.ei.length === 2 && keyframe.ei.every(isFiniteNumber)
    && typeof keyframe.hold === 'boolean'
    && typeof keyframe.i === 'string');
}

function isEffect(value: unknown): boolean {
  return isRecord(value)
    && typeof value.id === 'string'
    && typeof value.type === 'string'
    && typeof value.on === 'boolean'
    && (value.open === undefined || typeof value.open === 'boolean')
    && isRecord(value.p)
    && Object.values(value.p).every(isChannel);
}

function isTransition(value: unknown): boolean {
  return value === null || isRecord(value)
    && typeof value.type === 'string' && value.type.length > 0
    && isFiniteNumber(value.dur)
    && (value.missing === undefined || typeof value.missing === 'boolean')
    && isRecord(value.p)
    && Object.values(value.p).every(isChannel);
}

function isMask(value: unknown): boolean {
  if (!isRecord(value) || typeof value.id !== 'string'
      || !['rect', 'ellipse'].includes(String(value.shape))
      || !['add', 'subtract'].includes(String(value.mode))
      || typeof value.on !== 'boolean' || !isRecord(value.p)) return false;
  const parameters = value.p;
  return ['x', 'y', 'w', 'h', 'rotation', 'feather'].every(key => isChannel(parameters[key]));
}

function isLayerContent(type: LayerType, value: unknown): boolean {
  if (!isRecord(value)) return false;
  const numberFields = (...keys: string[]): boolean => keys.every(key => isFiniteNumber(value[key]));
  switch (type) {
    case 'solid': return typeof value.color === 'string' && numberFields('w', 'h', 'radius');
    case 'text': return typeof value.text === 'string' && typeof value.font === 'string'
      && numberFields('weight', 'size', 'tracking', 'leading') && typeof value.color === 'string'
      && ['left', 'center', 'right'].includes(String(value.align)) && typeof value.italic === 'boolean';
    case 'shape': return ['rect', 'ellipse', 'polygon', 'star', 'line'].includes(String(value.shape))
      && typeof value.color === 'string' && typeof value.strokeColor === 'string'
      && numberFields('w', 'h', 'radius', 'stroke', 'points');
    case 'image': return (value.asset === null || typeof value.asset === 'string')
      && ['cover', 'contain', 'stretch'].includes(String(value.fit)) && numberFields('w', 'h');
    case 'video': return (value.asset === null || typeof value.asset === 'string')
      && ['cover', 'contain', 'stretch'].includes(String(value.fit)) && numberFields('w', 'h', 'trim', 'speed');
    case 'audio': return (value.asset === null || typeof value.asset === 'string')
      && numberFields('gain', 'trim', 'fadeIn', 'fadeOut');
    case 'adjustment': return true;
    case 'shader': return typeof value.code === 'string' && numberFields('w', 'h')
      && isRecord(value.uniforms) && Object.values(value.uniforms).every(isChannel);
    case 'extension': return typeof value.definition === 'string'
      && Number.isSafeInteger(value.version) && Number(value.version) >= 1
      && numberFields('w', 'h')
      && isRecord(value.params) && Object.values(value.params).every(isChannel)
      && isRecord(value.data);
    case 'null': return true;
    case 'precomp': return (value.comp === null || typeof value.comp === 'string') && numberFields('w', 'h');
  }
}

const TRANSFORM_CHANNELS = [
  'anchor.x', 'anchor.y', 'position.x', 'position.y', 'scale.x', 'scale.y',
  'rotation', 'opacity', 'skew'
] as const;

function isLayer(value: unknown): value is Layer {
  if (!isRecord(value) || typeof value.type !== 'string' || !LAYER_TYPES.has(value.type as LayerType)) return false;
  const type = value.type as LayerType;
  if (typeof value.id !== 'string' || typeof value.name !== 'string'
      || !isFiniteNumber(value.from) || !isFiniteNumber(value.dur)
      || typeof value.on !== 'boolean' || typeof value.lock !== 'boolean'
      || typeof value.shy !== 'boolean'
      || typeof value.collapsed !== 'boolean' || typeof value.color !== 'string'
      || typeof value.blend !== 'string' || !BLEND_MODES.has(value.blend)
      || typeof value.mblur !== 'boolean'
      || !(value.parent === null || typeof value.parent === 'string')
      || !isRecord(value.p) || !Array.isArray(value.fx) || !value.fx.every(isEffect)
      || value.transitionIn !== undefined && !isTransition(value.transitionIn)
      || value.transitionOut !== undefined && !isTransition(value.transitionOut)
      || !Array.isArray(value.masks) || !value.masks.every(isMask)
      || !isLayerContent(type, value.d) || !isRecord(value.locked_intent)) return false;
  const properties = value.p;
  if (type === 'audio') return Object.keys(properties).length === 0;
  return TRANSFORM_CHANNELS.every(key => isChannel(properties[key]))
    && Object.values(properties).every(isChannel);
}

function parseLayers(value: unknown, path: string, requireNonEmpty: boolean): Layer[] | ValidationError {
  if (!Array.isArray(value) || (requireNonEmpty && value.length === 0)) {
    return invalid(requireNonEmpty ? 'must be a non-empty array' : 'must be an array', path);
  }
  const layers = cloneJson(value, path);
  if (layers instanceof ValidationError || !Array.isArray(layers)) {
    return layers instanceof ValidationError ? layers : invalid('must be an array', path);
  }
  for (const [index, layer] of layers.entries()) {
    if (!isLayer(layer)) return invalid('must be a complete serialized layer', `${path}[${index}]`);
  }
  return layers as unknown as Layer[];
}

function parseSectionVersion(value: unknown, path: string): SectionVersion | ValidationError {
  const version = jsonRecord(value, path);
  if (version instanceof ValidationError) return version;
  const id = requiredString(version.id, `${path}.id`);
  if (id instanceof ValidationError) return id;
  const layers = parseLayers(version.layers, `${path}.layers`, false);
  if (layers instanceof ValidationError) return layers;
  const at = finite(version.at, `${path}.at`);
  if (at instanceof ValidationError) return at;
  const out: SectionVersion = { id, layers, at };
  if (version.schemaVersion != null) {
    const schemaVersion = finite(version.schemaVersion, `${path}.schemaVersion`);
    if (schemaVersion instanceof ValidationError) return schemaVersion;
    out.schemaVersion = schemaVersion;
  }
  if (version.thumb === null || typeof version.thumb === 'string') out.thumb = version.thumb;
  else if (version.thumb !== undefined) return invalid('must be a string or null', `${path}.thumb`);
  return out;
}

function parseCreateSection(source: Record<string, unknown>): CreateSectionCommand | ValidationError {
  const section = jsonRecord(source.section, 'section');
  if (section instanceof ValidationError) return section;
  const id = requiredString(section.id, 'section.id');
  if (id instanceof ValidationError) return id;
  if (section.layers !== undefined) {
    const layers = parseLayers(section.layers, 'section.layers', false);
    if (layers instanceof ValidationError) return layers;
  }
  if (section.versions !== undefined) {
    if (!Array.isArray(section.versions)) return invalid('must be an array', 'section.versions');
    for (const [index, version] of section.versions.entries()) {
      const parsed = parseSectionVersion(version, `section.versions[${index}]`);
      if (parsed instanceof ValidationError) return parsed;
    }
  }
  return { type: 'create_section', section: section as unknown as SectionManifest };
}

function parseUpdateSection(source: Record<string, unknown>): UpdateSectionCommand | ValidationError {
  const sectionId = requiredString(source.sectionId, 'sectionId');
  if (sectionId instanceof ValidationError) return sectionId;
  const layers = parseLayers(source.layers, 'layers', true);
  if (layers instanceof ValidationError) return layers;
  const out: UpdateSectionCommand = { type: 'update_section', sectionId, layers };
  if (source.thumb === null || typeof source.thumb === 'string') out.thumb = source.thumb;
  else if (source.thumb !== undefined) return invalid('must be a string or null', 'thumb');
  if (source.at != null) {
    const at = finite(source.at, 'at');
    if (at instanceof ValidationError) return at;
    out.at = at;
  }
  if (source.version !== undefined) {
    const version = parseSectionVersion(source.version, 'version');
    if (version instanceof ValidationError) return version;
    out.version = version;
  }
  return out;
}

const pathAllowed = (path: string): path is TransformPath => LAYER_PATHS.has(path)
  || /^properties\.[a-z0-9_.-]{1,120}$/i.test(path)
  || /^content\.[a-z0-9_.-]{1,120}$/i.test(path);

function oneToEightArgs(args: TransformExpression[]): OneToEightTransformArgs {
  switch (args.length) {
    case 1: return [args[0]!];
    case 2: return [args[0]!, args[1]!];
    case 3: return [args[0]!, args[1]!, args[2]!];
    case 4: return [args[0]!, args[1]!, args[2]!, args[3]!];
    case 5: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!];
    case 6: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!, args[5]!];
    case 7: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!, args[5]!, args[6]!];
    default: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!, args[5]!, args[6]!, args[7]!];
  }
}

function twoToEightArgs(args: TransformExpression[]): TwoToEightTransformArgs {
  switch (args.length) {
    case 2: return [args[0]!, args[1]!];
    case 3: return [args[0]!, args[1]!, args[2]!];
    case 4: return [args[0]!, args[1]!, args[2]!, args[3]!];
    case 5: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!];
    case 6: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!, args[5]!];
    case 7: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!, args[5]!, args[6]!];
    default: return [args[0]!, args[1]!, args[2]!, args[3]!, args[4]!, args[5]!, args[6]!, args[7]!];
  }
}

function transformOperation(
  op: TransformOp,
  args: TransformExpression[]
): TransformOperationExpression | ValidationError {
  const arity = OP_ARITY[op]!;
  if (args.length < arity[0] || args.length > arity[1]) {
    return invalid('has the wrong operation arity', 'transform');
  }
  switch (op) {
    case 'add':
    case 'multiply':
      return { op, args: twoToEightArgs(args) };
    case 'subtract':
    case 'divide':
    case 'equal':
      return { op, args: [args[0]!, args[1]!] };
    case 'min':
    case 'max':
      return { op, args: oneToEightArgs(args) };
    case 'clamp':
    case 'if':
      return { op, args: [args[0]!, args[1]!, args[2]!] };
    case 'round':
    case 'floor':
    case 'ceil':
    case 'abs':
    case 'negate':
    case 'frames':
      return { op, args: [args[0]!] };
  }
}

function transformExpression(
  value: unknown,
  budget = { nodes: 0 },
  depth = 0
): TransformExpression | ValidationError {
  budget.nodes += 1;
  if (budget.nodes > 160 || depth > 8) return invalid('expression is too complex', 'transform');
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (!isRecord(value)) return invalid('contains an invalid expression', 'transform');
  if (typeof value.state === 'string' && SAFE_STATE_KEY.test(value.state)) {
    const out: { state: string; fallback?: TransformPrimitive } = { state: value.state };
    if (owns(value, 'fallback')) {
      const fallback = value.fallback;
      if (fallback === null || typeof fallback === 'string' || typeof fallback === 'boolean'
          || (typeof fallback === 'number' && Number.isFinite(fallback))) {
        out.fallback = fallback;
      }
    }
    return out;
  }
  if (typeof value.ref === 'string' && REFS.has(value.ref as TransformRef)) {
    return { ref: value.ref as TransformRef };
  }
  if (typeof value.aggregate === 'string' && AGGREGATES.has(value.aggregate as TransformAggregate)
      && typeof value.path === 'string' && pathAllowed(value.path)) {
    return { aggregate: value.aggregate as TransformAggregate, path: value.path };
  }
  if (typeof value.op === 'string' && OPS.has(value.op as TransformOp)) {
    const argsSource = Array.isArray(value.args) ? value.args.slice(0, 8) : [];
    const args: TransformExpression[] = [];
    for (const argSource of argsSource) {
      const arg = transformExpression(argSource, budget, depth + 1);
      if (arg instanceof ValidationError) return arg;
      args.push(arg);
    }
    return transformOperation(value.op as TransformOp, args);
  }
  return invalid('contains an invalid expression', 'transform');
}

function parseTransform(value: unknown): Transform | ValidationError {
  let source = value;
  if (typeof source === 'string') {
    if (source.length > 80_000) return invalid('exceeds 80000 JSON characters', 'transform');
    try { source = JSON.parse(source); } catch { return invalid('must be valid JSON', 'transform'); }
  }
  if (!isRecord(source)) return invalid('must be an object', 'transform');
  const selectorSource = isRecord(source.selector) ? source.selector : {};
  const scope: TransformScope = typeof selectorSource.scope === 'string'
    && SCOPES.has(selectorSource.scope as TransformScope)
    ? selectorSource.scope as TransformScope : 'selection';
  const types = (Array.isArray(selectorSource.types) ? selectorSource.types : [])
    .filter((type): type is LayerType => typeof type === 'string' && LAYER_TYPES.has(type as LayerType))
    .slice(0, 16);
  const edits: TransformEdit[] = [];
  for (const edit of (Array.isArray(source.edits) ? source.edits : []).slice(0, 32)) {
    if (!isRecord(edit) || typeof edit.path !== 'string') continue;
    const path = edit.path.trim().slice(0, 140);
    if (!pathAllowed(path)) continue;
    const expression = transformExpression(edit.value);
    if (expression instanceof ValidationError) continue;
    edits.push({ path, value: expression });
  }
  if (!edits.length) return invalid('must contain at least one valid edit', 'transform.edits');
  let order: TransformOrder | TransformStateOrder = 'stack';
  if (typeof source.order === 'string' && ORDERS.has(source.order as TransformOrder)) {
    order = source.order as TransformOrder;
  }
  else if (isRecord(source.order) && typeof source.order.state === 'string' && SAFE_STATE_KEY.test(source.order.state)) {
    order = {
      state: source.order.state,
      fallback: typeof source.order.fallback === 'string'
        && ORDERS.has(source.order.fallback as TransformOrder)
        ? source.order.fallback as TransformOrder : 'stack'
    };
  }
  const label = typeof source.label === 'string' && source.label.trim()
    ? source.label.trim().slice(0, 80) : 'Transform layers';
  const seed = typeof source.seed === 'string' && source.seed.trim()
    ? source.seed.trim().slice(0, 80) : 'powermove';
  return {
    version: 1,
    label,
    selector: { scope, includeLocked: false, types },
    order,
    seed,
    edits
  };
}

function parseTransformLayers(source: Record<string, unknown>): TransformLayersCommand | ValidationError {
  const transform = parseTransform(source.transform);
  if (transform instanceof ValidationError) return transform;
  const out: TransformLayersCommand = { type: 'transform_layers', transform };
  if (source.state !== undefined) {
    const state = jsonRecord(source.state, 'state');
    if (state instanceof ValidationError) return state;
    out.state = state;
  }
  return out;
}

function parseObject(source: Record<string, unknown>): EditCommand | ValidationError {
  if (typeof source.type !== 'string' || !COMMAND_TYPE_SET.has(source.type)) {
    return invalid('unknown edit command type', 'type');
  }
  switch (source.type) {
    case 'set_property': return parseSetProperty(source);
    case 'replace_keyframes': return parseReplaceKeyframes(source);
    case 'set_easing': return parseSetEasing(source);
    case 'set_expression': return parseSetExpression(source);
    case 'set_content': return parseSetContent(source);
    case 'set_layer': return parseSetLayer(source);
    case 'set_composition': return parseSetComposition(source);
    case 'add_layer': return parseAddLayer(source);
    case 'delete_layers': return parseDeleteLayers(source);
    case 'reorder_layer': return parseReorderLayer(source);
    case 'add_effect': return parseAddEffect(source);
    case 'remove_effect': return parseRemoveEffect(source);
    case 'set_effect': return parseSetEffect(source);
    case 'set_transition': return parseSetTransition(source);
    case 'set_scene_parameter': return parseSetSceneParameter(source);
    case 'add_marker': return parseAddMarker(source);
    case 'create_section': return parseCreateSection(source);
    case 'update_section': return parseUpdateSection(source);
    case 'transform_layers': return parseTransformLayers(source);
    default: return invalid('unknown edit command type', 'type');
  }
}

export function parseEditCommand(raw: unknown): EditCommand | ValidationError {
  let source = raw;
  if (typeof raw === 'string') {
    if (raw.length > COMMAND_JSON_LIMIT) return invalid(`command exceeds ${COMMAND_JSON_LIMIT} JSON characters`);
    try { source = JSON.parse(raw); } catch { return invalid('command must be valid JSON'); }
  }
  if (!isRecord(source)) return invalid('edit command must be an object');
  const preflightError = preflightSelectedSize(source);
  if (preflightError) return preflightError;
  const command = parseObject(source);
  return command instanceof ValidationError ? command : checkedSize(command);
}

/**
 * Agent proposals use the narrower allowlist from assistant/harness.js and can
 * never bypass locks or hand-authored intent. Editor/UI callers should use
 * parseEditCommand so legitimate provenance fields survive unchanged.
 */
export function parseAgentEditCommand(raw: unknown): EditCommand | ValidationError {
  let source = raw;
  if (typeof raw === 'string') {
    if (raw.length > COMMAND_JSON_LIMIT) return invalid(`command exceeds ${COMMAND_JSON_LIMIT} JSON characters`);
    try { source = JSON.parse(raw); } catch { return invalid('command must be valid JSON'); }
  }
  if (!isRecord(source) || typeof source.type !== 'string' || !AGENT_COMMAND_TYPE_SET.has(source.type)) {
    return invalid('unknown agent edit command type', 'type');
  }
  const selected: Record<string, unknown> = {};
  const fields = AGENT_COMMAND_FIELDS[source.type as keyof typeof AGENT_COMMAND_FIELDS];
  for (const key of fields) if (source[key] !== undefined) selected[key] = source[key];
  const command = parseEditCommand(selected);
  if (command instanceof ValidationError) return command;
  if (command.type === 'set_property' || command.type === 'replace_keyframes') {
    command.preserveHandEdits = true;
  }
  return command;
}

export interface ParseEditCommandsOptions {
  maxCommands?: number;
  /** Legacy sanitizeProposal skips malformed members and keeps the valid remainder. */
  onInvalid?: 'skip' | 'reject';
  source?: 'editor' | 'agent';
}

export function parseEditCommands(
  raw: unknown,
  options: ParseEditCommandsOptions = {}
): EditCommand[] | ValidationError {
  let source = raw;
  if (typeof raw === 'string') {
    try { source = JSON.parse(raw); } catch { return invalid('commands must be valid JSON'); }
  }
  if (!Array.isArray(source)) return invalid('commands must be an array');
  const requested = options.maxCommands ?? MAX_COMMANDS;
  if (!Number.isSafeInteger(requested) || requested < 0) return invalid('maxCommands must be a non-negative integer');
  const commands: EditCommand[] = [];
  const parser = options.source === 'agent' ? parseAgentEditCommand : parseEditCommand;
  for (const [index, item] of source.slice(0, requested).entries()) {
    const command = parser(item);
    if (command instanceof ValidationError) {
      if (options.onInvalid === 'reject') return invalid(command.message, `commands[${index}]`);
      continue;
    }
    commands.push(command);
  }
  return commands;
}
