import { CHANNELS_3D } from '../../legacy/core/space-3d';
import { canAnimateContent, isProperty } from '../../legacy/core/content-properties';
import { adoptTemporalEase } from '../anim/temporal-ease';
import { compactEditLog } from '../edit-log';
import {
  BLEND_MODES,
  TYPE_META,
  type AudioContent,
  type Channel,
  type ChannelValue,
  type Comp,
  type Effect,
  type EditLogEntry,
  type EditLogOperation,
  type Fill,
  type Layer,
  type LayerType,
  type Mask,
  type Project,
  type ProjectJsonValue,
  type SceneParam,
  type Transition,
  type TransformChannels
} from '../types/project';

type UnknownRecord = Record<string, unknown>;

const TRANSFORM_DEFAULTS = {
  ...CHANNELS_3D,
  'anchor.x': 0,
  'anchor.y': 0,
  'position.x': 0,
  'position.y': 0,
  'scale.x': 100,
  'scale.y': 100,
  rotation: 0,
  opacity: 100,
  skew: 0
} as const;

const MASK_DEFAULTS = { x: 0, y: 0, w: 0, h: 0, rotation: 0, feather: 24 } as const;
const MASK_SHAPES = ['rect', 'ellipse'] as const;
const FONT_AXIS_PREFIX = 'fontAxis.';
const FORBIDDEN_KEYS = new Set(['__proto__', 'prototype', 'constructor']);
const EFFECT_TYPES = new Set([
  'blur', 'motionblurDir', 'sharpen', 'glow', 'color', 'levels', 'duotone', 'grain',
  'vignette', 'chroma', 'pixelate', 'posterize', 'displace', 'shadow', 'invert'
]);

let nextId = 0;
const uid = (prefix: string): string => `${prefix}-${Date.now().toString(36)}-${(++nextId).toString(36)}`;
const isRecord = (value: unknown): value is UnknownRecord =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const finite = (value: unknown, fallback = 0): number => {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
};
const clamp = (value: number, min: number, max: number): number => Math.max(min, Math.min(max, value));
const stringOr = (value: unknown, fallback: string): string =>
  typeof value === 'string' ? value : fallback;
const nonEmptyStringOr = (value: unknown, fallback: string): string =>
  typeof value === 'string' && value ? value : fallback;

function copyRecord(value: unknown): UnknownRecord {
  return isRecord(value) ? { ...value } : {};
}

function normalizeFill(value: unknown, fallback = '#000000'): Fill {
  const color = (candidate: unknown): string =>
    typeof candidate === 'string' && /^#[0-9a-f]{6}$/i.test(candidate)
      ? candidate.toUpperCase()
      : fallback;
  const raw = isRecord(value) ? value : {};
  const fillTypes = ['solid', 'linear', 'radial', 'none'] as const;
  const type = fillTypes.includes(raw.type as Fill['type']) ? raw.type as Fill['type'] : 'solid';
  let stops = (Array.isArray(raw.stops) ? raw.stops : []).slice(0, 8).map((candidate, index) => {
    const stop = isRecord(candidate) ? candidate : {};
    return {
      id: typeof stop.id === 'string' && stop.id ? stop.id : `stop-${index + 1}`,
      color: color(stop.color),
      position: clamp(Number(stop.position) || 0, 0, 100)
    };
  }).sort((a, b) => a.position - b.position);
  if (!stops.length) {
    stops = [{ id: 'stop-1', color: color(raw.color ?? fallback), position: 0 }];
  }
  if (type !== 'solid' && type !== 'none' && stops.length < 2) {
    stops.push({ id: 'stop-2', color: stops[0]!.color, position: 100 });
  }
  if (type === 'solid' || type === 'none') stops = [{ ...stops[0]!, position: 0 }];
  return { type, angle: clamp(Number(raw.angle) || 0, -180, 180), stops };
}

function channel<T extends ChannelValue>(value: T): Channel<T> {
  return { v: value, kf: [], expr: null };
}

function sanitizeChannel(raw: unknown, fallback: number): Channel<number> {
  const prop = copyRecord(raw);
  const sourceKeys = Array.isArray(prop.kf) ? prop.kf : [];
  const keys = sourceKeys
    .filter((item): item is UnknownRecord => {
      if (!isRecord(item)) return false;
      return Number.isFinite(Number(item.t)) && Number.isFinite(Number(item.v));
    })
    .map(item => ({
      ...item,
      t: Number(item.t),
      v: Number(item.v),
      i: typeof item.i === 'string' && item.i ? item.i : uid('k')
    }))
    .sort((a, b) => a.t - b.t)
    .filter((item, index, all) => index === 0 || item.t - all[index - 1]!.t > 1e-6);
  return {
    v: finite(prop.v, fallback),
    kf: adoptTemporalEase(keys),
    expr: typeof prop.expr === 'string' && prop.expr.trim() ? prop.expr : null
  };
}

function isChannelValue(value: unknown): value is ChannelValue {
  return typeof value === 'number' && Number.isFinite(value)
    || typeof value === 'string'
    || typeof value === 'boolean';
}

function sanitizeLooseChannel(raw: unknown, fallback: ChannelValue): Channel {
  const prop = copyRecord(raw);
  const staticValue = isChannelValue(prop.v) ? prop.v : fallback;
  const keys = (Array.isArray(prop.kf) ? prop.kf : [])
    .filter((item): item is UnknownRecord => isRecord(item)
      && Number.isFinite(Number(item.t))
      && isChannelValue(item.v))
    .map(item => ({
      ...item,
      t: Number(item.t),
      v: item.v as ChannelValue,
      i: typeof item.i === 'string' && item.i ? item.i : uid('k')
    }))
    .sort((a, b) => a.t - b.t)
    .filter((item, index, all) => index === 0 || item.t - all[index - 1]!.t > 1e-6);
  return {
    v: staticValue,
    kf: adoptTemporalEase(keys),
    expr: typeof prop.expr === 'string' && prop.expr.trim() ? prop.expr : null
  };
}

function freshTransformChannels(type: LayerType, comp: Pick<Comp, 'w' | 'h'>): TransformChannels {
  const defaults = {
    ...TRANSFORM_DEFAULTS,
    'position.x': comp.w / 2,
    'position.y': comp.h / 2
  };
  if (type === 'group' || type === 'solid' || type === 'adjustment' || type === 'shader' || type === 'extension' || type === 'precomp') {
    defaults['position.x'] = 0;
    defaults['position.y'] = 0;
  }
  return Object.fromEntries(
    Object.entries(defaults).map(([name, value]) => [name, channel(value)])
  ) as unknown as TransformChannels;
}

function sanitizeTransformChannels(raw: unknown, type: LayerType, comp: Pick<Comp, 'w' | 'h'>): TransformChannels {
  const source = copyRecord(raw);
  const fresh = freshTransformChannels(type, comp);
  for (const name of Object.keys(fresh) as Array<keyof TransformChannels>) {
    fresh[name] = sanitizeChannel(source[name], fresh[name].v);
  }
  return fresh;
}

function sanitizeEffect(raw: unknown): Effect | null {
  if (!isRecord(raw) || typeof raw.type !== 'string' || !EFFECT_TYPES.has(raw.type)) return null;
  const parameters: Record<string, Channel> = {};
  if (isRecord(raw.p)) {
    for (const [name, value] of Object.entries(raw.p)) {
      if (FORBIDDEN_KEYS.has(name) || !isRecord(value)) continue;
      parameters[name] = sanitizeLooseChannel(value, 0);
    }
  }
  const effect: Effect = {
    id: typeof raw.id === 'string' && raw.id ? raw.id : uid('fx'),
    type: raw.type,
    on: (isProperty(raw.on) ? sanitizeLooseChannel(raw.on, true) : raw.on !== false) as any,
    p: parameters
  };
  if ('open' in raw) effect.open = !!raw.open;
  return effect;
}

function isTransitionParamValue(value: unknown): value is ChannelValue {
  return typeof value === 'number' && Number.isFinite(value)
    || typeof value === 'boolean'
    || typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value);
}

function sanitizeTransitionChannel(raw: unknown): Channel {
  const prop = copyRecord(raw);
  const staticValue = isTransitionParamValue(prop.v) ? prop.v : 0;
  const keys = (Array.isArray(prop.kf) ? prop.kf : [])
    .filter((item): item is UnknownRecord => isRecord(item)
      && Number.isFinite(Number(item.t))
      && isTransitionParamValue(item.v))
    .map(item => ({
      ...item,
      t: Number(item.t),
      v: item.v as ChannelValue,
      i: typeof item.i === 'string' && item.i ? item.i : uid('k')
    }))
    .sort((a, b) => a.t - b.t)
    .filter((item, index, all) => index === 0 || item.t - all[index - 1]!.t > 1e-6);
  return {
    v: staticValue,
    kf: adoptTemporalEase(keys),
    expr: typeof prop.expr === 'string' && prop.expr.trim() ? prop.expr : null
  };
}

function sanitizeTransition(raw: unknown): Transition | null {
  if (!isRecord(raw) || typeof raw.type !== 'string' || !raw.type) return null;
  const parameters: Record<string, Channel> = {};
  if (isRecord(raw.p)) {
    for (const [name, value] of Object.entries(raw.p)) {
      if (FORBIDDEN_KEYS.has(name) || !isRecord(value)) continue;
      parameters[name] = sanitizeTransitionChannel(value);
    }
  }
  const transition: Transition = {
    type: raw.type,
    dur: clamp(finite(raw.dur, 0.5), 0.02, 600),
    p: parameters
  };
  if (raw.missing === true) transition.missing = true;
  return transition;
}

function sanitizeMask(raw: unknown, comp: Pick<Comp, 'w' | 'h'>): Mask | null {
  if (!isRecord(raw) || !isRecord(raw.p)) return null;
  const shape = MASK_SHAPES.includes(raw.shape as Mask['shape']) ? raw.shape as Mask['shape'] : 'rect';
  const size = Math.round(Math.min(comp.w, comp.h) * 0.5);
  const defaults = { ...MASK_DEFAULTS, w: size, h: size };
  const source = raw.p;
  return {
    id: typeof raw.id === 'string' && raw.id ? raw.id : uid('K'),
    ...(isRecord(raw.path) ? {path: sanitizeContentRecord(raw.path) as any} : {}),
    shape: (isProperty(raw.shape) ? sanitizeLooseChannel(raw.shape, 'rect') : shape) as any,
    mode: (isProperty(raw.mode) ? sanitizeLooseChannel(raw.mode, 'add') : raw.mode === 'subtract' ? 'subtract' : 'add') as any,
    on: (isProperty(raw.on) ? sanitizeLooseChannel(raw.on, true) : raw.on !== false) as any,
    p: {
      x: sanitizeChannel(source.x, defaults.x),
      y: sanitizeChannel(source.y, defaults.y),
      w: sanitizeChannel(source.w, defaults.w),
      h: sanitizeChannel(source.h, defaults.h),
      rotation: sanitizeChannel(source.rotation, defaults.rotation),
      feather: sanitizeChannel(source.feather, defaults.feather)
    }
  };
}

function normalizeAudioContent(raw: unknown): AudioContent {
  const source = sanitizeContentRecord(raw);
  return {
    asset: typeof source.asset === 'string' && source.asset ? source.asset : null,
    trim: Math.max(0, finite(source.trim)),
    gain: clamp(finite(source.gain, 1), 0, 4),
    fadeIn: Math.max(0, finite(source.fadeIn)),
    fadeOut: Math.max(0, finite(source.fadeOut))
  };
}

const OMIT_JSON_VALUE = Symbol('omit-json-value');

function sanitizeJsonValue(
  value: unknown,
  ancestors: Set<object>
): ProjectJsonValue | typeof OMIT_JSON_VALUE {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number') return Number.isFinite(value) ? value : OMIT_JSON_VALUE;
  if (typeof value !== 'object') return OMIT_JSON_VALUE;
  if (ancestors.has(value)) return OMIT_JSON_VALUE;
  ancestors.add(value);
  if (Array.isArray(value)) {
    const output: ProjectJsonValue[] = [];
    for (const item of value) {
      const clean = sanitizeJsonValue(item, ancestors);
      if (clean !== OMIT_JSON_VALUE) output.push(clean);
    }
    ancestors.delete(value);
    return output;
  }
  if (Object.prototype.toString.call(value) !== '[object Object]') {
    ancestors.delete(value);
    return OMIT_JSON_VALUE;
  }
  const output: Record<string, ProjectJsonValue> = {};
  for (const [key, item] of Object.entries(value)) {
    if (FORBIDDEN_KEYS.has(key)) continue;
    const clean = sanitizeJsonValue(item, ancestors);
    if (clean !== OMIT_JSON_VALUE) output[key] = clean;
  }
  ancestors.delete(value);
  return output;
}

function sanitizeContentRecord(raw: unknown): UnknownRecord {
  if (!isRecord(raw)) return {};
  const clean = sanitizeJsonValue(raw, new Set());
  return isRecord(clean) ? clean : {};
}

function sanitizeTextFontAxes(source: UnknownRecord): UnknownRecord {
  const output: UnknownRecord = {};
  for (const [key, value] of Object.entries(source)) {
    if (!key.startsWith(FONT_AXIS_PREFIX) || !isRecord(value)) continue;
    const axis = key.slice(FONT_AXIS_PREFIX.length);
    if (!/^[\x20-\x7e]{4}$/.test(axis)) continue;
    output[key] = sanitizeChannel(value, finite(value.v));
  }
  return output;
}

function sanitizeLockedIntent(raw: unknown): Record<string, UnknownRecord> {
  if (!isRecord(raw)) return {};
  return Object.fromEntries(Object.entries(raw).filter((entry): entry is [string, UnknownRecord] =>
    !FORBIDDEN_KEYS.has(entry[0]) && isRecord(entry[1])));
}

function contentFor(type: LayerType, raw: unknown, comp: Pick<Comp, 'w' | 'h'>): Layer['d'] {
  const source = sanitizeContentRecord(raw);
  const values = { ...source };
  for (const [key, value] of Object.entries(source)) {
    if (!key.startsWith(FONT_AXIS_PREFIX) && canAnimateContent({ type }, key) && isProperty(value)) values[key] = (value as any).v;
  }
  const output = staticContentFor(type, values, comp) as any;
  for (const [key, value] of Object.entries(source)) {
    if (!key.startsWith(FONT_AXIS_PREFIX) && canAnimateContent({ type }, key) && isProperty(value)) output[key] = sanitizeLooseChannel(value, isProperty(output[key]) ? output[key].v : output[key]);
  }
  return output;
}

function staticContentFor(type: LayerType, raw: unknown, comp: Pick<Comp, 'w' | 'h'>): Layer['d'] {
  const source = sanitizeContentRecord(raw);
  switch (type) {
    case 'solid':
      return {
        ...source,
        color: stringOr(source.color, '#1b1b1f'),
        w: finite(source.w, comp.w), h: finite(source.h, comp.h), radius: finite(source.radius)
      };
    case 'text':
      return {
        ...source,
        ...sanitizeTextFontAxes(source),
        text: stringOr(source.text, 'Powermove'), font: stringOr(source.font, 'SF Pro Display'),
        boxWidth: sanitizeChannel(source.boxWidth, 0), boxHeight: sanitizeChannel(source.boxHeight, 0),
        weight: finite(source.weight, 600), size: finite(source.size, 128),
        tracking: finite(source.tracking, -2), leading: finite(source.leading, 1.1),
        color: stringOr(source.color, '#F2F2F2'),
        align: source.align === 'left' || source.align === 'right' ? source.align : 'center',
        italic: !!source.italic
      };
    case 'shape': {
      const shapes = ['rect', 'ellipse', 'polygon', 'star', 'line'] as const;
      return {
        ...source,
        shape: shapes.includes(source.shape as (typeof shapes)[number])
          ? source.shape as (typeof shapes)[number] : 'rect',
        color: stringOr(source.color, '#E8E2CF'), w: finite(source.w, 480), h: finite(source.h, 480),
        radius: finite(source.radius, 24), stroke: finite(source.stroke),
        strokeColor: stringOr(source.strokeColor, '#ffffff'), points: finite(source.points, 5)
      };
    }
    case 'image':
      return {
        ...source,
        asset: typeof source.asset === 'string' && source.asset ? source.asset : null,
        fit: source.fit === 'contain' || source.fit === 'stretch' ? source.fit : 'cover',
        w: finite(source.w, 1920), h: finite(source.h, 1080)
      };
    case 'video':
      return {
        ...source,
        asset: typeof source.asset === 'string' && source.asset ? source.asset : null,
        fit: source.fit === 'contain' || source.fit === 'stretch' ? source.fit : 'cover',
        trim: finite(source.trim), speed: finite(source.speed, 1),
        embeddedAudio: source.embeddedAudio === true,
        w: finite(source.w, 1920), h: finite(source.h, 1080)
      };
    case 'audio':
      return normalizeAudioContent(source);
    case 'adjustment':
      return source;
    case 'shader':
      return {
        ...source,
        code: stringOr(source.code, ''), w: finite(source.w, comp.w), h: finite(source.h, comp.h),
        uniforms: sanitizeShaderUniforms(source.code, source.uniforms)
      };
    case 'extension': {
      const rawParams = copyRecord(source.params);
      const params: Record<string, Channel> = {};
      for (const [name, value] of Object.entries(rawParams)) {
        if (FORBIDDEN_KEYS.has(name) || !/^[a-zA-Z][a-zA-Z0-9_]{0,63}$/.test(name)) continue;
        const record = copyRecord(value);
        const fallback = isChannelValue(record.v) ? record.v : 0;
        params[name] = sanitizeLooseChannel(value, fallback);
      }
      return {
        ...source,
        definition: stringOr(source.definition, ''),
        version: Math.max(1, Math.round(finite(source.version, 1))),
        w: Math.max(1, finite(source.w, comp.w)),
        h: Math.max(1, finite(source.h, comp.h)),
        params,
        data: sanitizeContentRecord(source.data) as Record<string, ProjectJsonValue>
      };
    }
    case 'precomp':
      return {
        ...source,
        comp: typeof source.comp === 'string' && source.comp ? source.comp : null,
        w: finite(source.w, comp.w), h: finite(source.h, comp.h)
      };
    case 'null':
      return {
        ...source,
        color: stringOr(source.color, '#6A6A70'),
        w: Math.max(1, finite(source.w, 100)),
        h: Math.max(1, finite(source.h, 100)),
        radius: Math.max(0, finite(source.radius))
      };
    case 'group':
      return source;
  }
}

function sanitizeShaderUniforms(code: unknown, raw: unknown): Record<string, Channel> {
  const source = copyRecord(raw);
  const output: Record<string, Channel> = {};
  if (typeof code !== 'string') return output;
  const expression = /uniform\s+(float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g;
  for (const match of code.matchAll(expression)) {
    const type = match[1]!;
    const name = match[2]!;
    if (FORBIDDEN_KEYS.has(name) || /^i(Resolution|Time|GlobalTime|Progress|Frame|Mouse)$/.test(name)) continue;
    const annotations = (match[3] || '').trim().split(/\s+/).filter(Boolean);
    let fallback: ChannelValue;
    if (type === 'vec3' && annotations[0]?.startsWith('#')) fallback = annotations[0];
    else if (type === 'bool') fallback = annotations[0] === 'true';
    else {
      const parsed = annotations[0] === undefined ? 0 : Number.parseFloat(annotations[0]);
      fallback = Number.isNaN(parsed) ? 0 : parsed;
    }
    const existing = source[name];
    output[name] = isRecord(existing) ? sanitizeLooseChannel(existing, fallback) : channel(fallback);
  }
  return output;
}

function sanitizeLayer(raw: unknown, index: number, comp: Pick<Comp, 'w' | 'h' | 'dur'>): Layer {
  const source = copyRecord(raw);
  const type: LayerType = typeof source.type === 'string' && source.type in TYPE_META
    ? source.type as LayerType
    : 'null';
  const transformable = type !== 'audio';
  const noEffects = type === 'audio';
  const noMasks = type === 'audio';
  const id = nonEmptyStringOr(source.id, uid('L'));
  const blend = BLEND_MODES.includes(source.blend as (typeof BLEND_MODES)[number])
    ? source.blend as (typeof BLEND_MODES)[number]
    : 'normal';
  const layer = {
    id,
    type,
    name: nonEmptyStringOr(source.name, `Layer ${index + 1}`),
    from: Math.max(type === 'group' ? -Infinity : 0, finite(source.from)),
    dur: Math.max(0.01, finite(source.dur, 5)),
    on: isProperty(source.on) ? sanitizeLooseChannel(source.on, true) : source.on !== false,
    lock: !!source.lock,
    shy: !!source.shy,
    threeD: type !== 'audio' && type !== 'adjustment' && !!source.threeD,
    solo: !!source.solo,
    matteSource: typeof source.matteSource === 'string' ? source.matteSource : null,
    matteMode: sanitizeLooseChannel(source.matteMode, 'alpha') as any,
    group: typeof source.group === 'string' && source.group !== id ? source.group : null,
    collapsed: source.collapsed !== false,
    color: stringOr(source.color, TYPE_META[type].color),
    blend: type === 'audio' ? 'normal' : isProperty(source.blend) ? sanitizeLooseChannel(source.blend, 'normal') : blend,
    mblur: type === 'audio' ? false : isProperty(source.mblur) ? sanitizeLooseChannel(source.mblur, false) : !!source.mblur,
    parent: type === 'audio' ? null : (typeof source.parent === 'string' && source.parent !== id ? source.parent : null),
    p: transformable ? sanitizeTransformChannels(source.p, type, comp) : {},
    fx: noEffects ? [] : (Array.isArray(source.fx) ? source.fx.map(sanitizeEffect).filter((item): item is Effect => item !== null) : []),
    transitionIn: sanitizeTransition(source.transitionIn),
    transitionOut: sanitizeTransition(source.transitionOut),
    masks: noMasks ? [] : (Array.isArray(source.masks)
      ? source.masks.map(mask => sanitizeMask(mask, comp)).filter((item): item is Mask => item !== null)
      : []),
    d: contentFor(type, source.d, comp),
    locked_intent: sanitizeLockedIntent(source.locked_intent)
  };
  return layer as Layer;
}

function sanitizeParams(raw: unknown): Record<string, SceneParam> {
  if (!isRecord(raw)) return {};
  const output: Record<string, SceneParam> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (FORBIDDEN_KEYS.has(key) || !isRecord(value)) continue;
    const common = {
      ...value,
      name: nonEmptyStringOr(value.name, key),
      label: nonEmptyStringOr(value.label, nonEmptyStringOr(value.name, key))
    };
    const control = value.control === 'color' || value.control === 'toggle' || value.control === 'select'
      ? value.control
      : 'num';
    if (control === 'toggle') output[key] = { ...common, control, value: !!value.value };
    else if (control === 'color') {
      output[key] = {
        ...common, control,
        value: typeof value.value === 'string' && /^#[0-9a-f]{6}$/i.test(value.value)
          ? value.value : '#FF6B1A'
      };
    } else if (control === 'select') {
      const options = Array.isArray(value.options)
        ? value.options.filter((option): option is UnknownRecord & { v: unknown } => isRecord(option) && 'v' in option)
        : [];
      output[key] = {
        ...common, control, options,
        value: value.value === undefined ? (options[0]?.v ?? '') : value.value
      };
    } else {
      let min = finite(value.min, 0);
      let max = finite(value.max, Math.max(min, 1));
      if (max < min) [min, max] = [max, min];
      output[key] = { ...common, control, min, max, value: clamp(finite(value.value, min), min, max) };
    }
  }
  return output;
}

function sanitizeMarkers(raw: unknown): Project['markers'] {
  if (!Array.isArray(raw)) return [];
  return raw.filter(isRecord).map(marker => {
    const result: Project['markers'][number] = {
      t: finite(marker.t),
      name: stringOr(marker.name, '')
    };
    if (typeof marker.id === 'string' && marker.id) result.id = marker.id;
    return result;
  });
}

function sanitizeEdits(raw: unknown): EditLogEntry[] {
  if (!Array.isArray(raw)) return [];
  return compactEditLog(raw.filter(isRecord)).map(entry => {
    const operations = (Array.isArray(entry.operations) ? entry.operations : [])
      .map(operation => sanitizeJsonValue(operation, new Set()))
      .filter((operation): operation is EditLogOperation => isRecord(operation));
    return {
      id: nonEmptyStringOr(entry.id, uid('edit')),
      revision: Math.max(0, finite(entry.revision)),
      at: finite(entry.at, Date.now()),
      origin: stringOr(entry.origin, 'interface'),
      label: stringOr(entry.label, 'Edit source'),
      summary: Array.isArray(entry.summary)
        ? entry.summary.filter((item): item is string => typeof item === 'string')
        : [],
      operations,
      ...(entry.payloadOmitted === true ? { payloadOmitted: true } : {})
    };
  });
}

function sanitizeComp(raw: unknown, id: string, root: Pick<Project, 'w' | 'h' | 'fps' | 'dur'>): Comp | null {
  if (!isRecord(raw)) return null;
  const comp = { ...raw } as UnknownRecord;
  const w = Math.max(2, finite(raw.w, root.w));
  const h = Math.max(2, finite(raw.h, root.h));
  const fps = Math.max(1, finite(raw.fps, root.fps));
  const dur = Math.max(0.04, finite(raw.dur, root.dur));
  const layers = Array.isArray(raw.layers) ? raw.layers.map((layer, index) => sanitizeLayer(layer, index, { w, h, dur })) : [];
  const result = {
    ...comp,
    id,
    name: nonEmptyStringOr(raw.name, 'Precomp'),
    w, h, fps, dur,
    bg: stringOr(raw.bg, '#000000'),
    backgroundFill: normalizeFill(raw.backgroundFill, stringOr(raw.bg, '#000000')),
    layers,
    comps: {},
    assets: isRecord(raw.assets) ? raw.assets as Comp['assets'] : {},
    markers: sanitizeMarkers(raw.markers),
    work: Array.isArray(raw.work) && raw.work.length === 2
      ? [finite(raw.work[0]), finite(raw.work[1], dur)] : [0, dur],
    params: sanitizeParams(raw.params),
    revision: finite(raw.revision),
    edits: sanitizeEdits(raw.edits),
    created: finite(raw.created, Date.now()),
    shutter: raw.shutter == null ? 0.5 : finite(raw.shutter, 0.5)
  } as Comp;
  if (isRecord(raw.comps)) {
    for (const [childId, childRaw] of Object.entries(raw.comps)) {
      if (FORBIDDEN_KEYS.has(childId)) continue;
      const child = sanitizeComp(childRaw, childId, result);
      if (child) result.comps[childId] = child;
    }
  }
  const ids = new Set(result.layers.map(layer => layer.id));
  for (const layer of result.layers) {
    if (layer.parent && !ids.has(layer.parent)) layer.parent = null;
  }
  return result;
}

/**
 * Sanitize a bare legacy project or a serialized `.pmv` envelope.
 *
 * This is intentionally not wired into loading yet. It follows app.js hydrate:
 * fill/params, channels, masks, effects, audio, and nested comps are normalized;
 * malformed layer content is additionally filled from mkLayer defaults so the
 * returned discriminated union is truthful at the TypeScript boundary.
 */
export function sanitizeProject(raw: unknown): Project {
  let input = raw;
  if (typeof input === 'string') {
    try { input = JSON.parse(input) as unknown; }
    catch { input = {}; }
  }
  if (isRecord(input) && isRecord(input.proj)) input = input.proj;
  const source = copyRecord(input);
  const w = finite(source.w, 1920) || 1920;
  const h = finite(source.h, 1080) || 1080;
  const fps = finite(source.fps, 30) || 30;
  const dur = finite(source.dur, 10) || 10;
  const fallbackBg = stringOr(source.bg, '#000000') || '#000000';
  const backgroundFill = normalizeFill(source.backgroundFill, fallbackBg);
  const project: Project = {
    ...source,
    id: nonEmptyStringOr(source.id, uid('P')),
    name: nonEmptyStringOr(source.name, 'Untitled'),
    w, h, fps, dur,
    bg: backgroundFill.stops[0]!.color,
    backgroundFill,
    layers: [],
    comps: {},
    assets: isRecord(source.assets) ? source.assets as Project['assets'] : {},
    markers: sanitizeMarkers(source.markers),
    work: Array.isArray(source.work) && source.work.length === 2
      ? [finite(source.work[0]), finite(source.work[1], dur)] : [0, dur],
    params: sanitizeParams(source.params),
    revision: finite(source.revision),
    edits: sanitizeEdits(source.edits),
    created: finite(source.created, Date.now()),
    shutter: source.shutter == null ? 0.5 : finite(source.shutter, 0.5)
  };
  if (isRecord(source.comps)) {
    for (const [id, value] of Object.entries(source.comps)) {
      if (FORBIDDEN_KEYS.has(id)) continue;
      const comp = sanitizeComp(value, id, project);
      if (comp) project.comps[id] = comp;
    }
  }
  project.layers = Array.isArray(source.layers)
    ? source.layers.map((layer, index) => sanitizeLayer(layer, index, project))
    : [];
  const ids = new Set(project.layers.map(layer => layer.id));
  const groups = new Map(project.layers.filter(layer => layer.type === 'group').map(layer => [layer.id, layer]));
  for (const layer of project.layers) {
    const seen = new Set([layer.id]); let id = layer.group;
    while (id) {
      if (seen.has(id) || !groups.has(id)) { layer.group = null; break; }
      seen.add(id); id = groups.get(id)?.group;
    }
  }
  for (const layer of project.layers) {
    if (layer.parent && !ids.has(layer.parent)) layer.parent = null;
    if (layer.type === 'precomp' && (!layer.d.comp || !project.comps[layer.d.comp])) layer.d.comp = null;
  }
  for (const comp of Object.values(project.comps)) {
    const compIds = new Set(comp.layers.map(layer => layer.id));
    for (const layer of comp.layers) if (layer.parent && !compIds.has(layer.parent)) layer.parent = null;
  }
  return project;
}
