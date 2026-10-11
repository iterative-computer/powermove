/**
 * Deterministic document builders. Every id is derived from a per-project
 * counter, every timestamp is fixed, and every layer carries the same fields
 * `PM.mkLayer` produces (`legacy/core/model.ts`), so generated documents look
 * like ones the app writes and are byte-stable across runs.
 */
import { animatorPresets, captionModel, effectDef, makeAnimator, scene3d, transitionDef, typeMeta } from './catalog';

export type Value = number | string | boolean;
export interface Key { t: number; v: Value; i: string; [field: string]: unknown }
export interface Channel { v: Value; kf: Key[]; expr: string | null }
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type Doc = Record<string, any>;
export type Layer = Record<string, any>;

/** Fixed creation time: 2026-10-11T00:00:00Z. Generated documents never read the clock. */
export const CREATED = 1791676800000;

export const CHANNELS_3D: Record<string, number> = {
  perspective: 50, 'position.z': 0, 'anchor.z': 0, 'scale.z': 100,
  'rotation.x': 0, 'rotation.y': 0, 'orientation.x': 0, 'orientation.y': 0, 'orientation.z': 0,
};

/** Legacy easing presets (`legacy/core/easing.ts` PRESETS) as [eo, ei] handle pairs. */
export const LEGACY_EASES: Record<string, [[number, number], [number, number]]> = {
  linear: [[0, 0], [1, 1]],
  easeInOut: [[.42, 0], [.58, 1]],
  cubicOut: [[.33, 1], [.68, 1]],
  expoOut: [[.16, 1], [.3, 1]],
  backOut: [[.34, 1.56], [.64, 1]],
  power: [[.62, .05], [0, 1]],
  snap: [[.9, 0], [.1, 1]],
};

export type KeySpec =
  | [number, Value]
  | [number, Value, KeyOptions];

export interface KeyOptions {
  /** Native After Effects temporal ease on both sides. */
  ease?: { inInterp?: 'linear' | 'bezier' | 'hold'; outInterp?: 'linear' | 'bezier' | 'hold';
    inEase?: { speed: number; influence: number }; outEase?: { speed: number; influence: number };
    autoBezier?: boolean; continuous?: boolean };
  /** Legacy eo/ei handles, adopted by the loader (`adoptTemporalEase`). */
  legacy?: keyof typeof LEGACY_EASES;
  hold?: boolean;
  spring?: { damping?: number; stiffness?: number; mass?: number };
}

export interface CompOptions {
  name: string; w?: number; h?: number; fps?: number; dur?: number; bg?: string;
  backgroundFill?: Doc; shutter?: number;
}

export class Builder {
  private counter = 0;
  /** Composition receiving new layers; channels default against its size. */
  comp: Doc;

  constructor(readonly slug: string, public root: Doc) {
    this.comp = root;
  }

  id(prefix: string): string {
    return `${prefix}${(++this.counter).toString(36).padStart(3, '0')}`;
  }

  ch(v: Value, keys?: KeySpec[], expr: string | null = null): Channel {
    return { v, kf: (keys ?? []).map(spec => this.key(spec[0], spec[1], spec[2])), expr };
  }

  key(t: number, v: Value, options: KeyOptions = {}): Key {
    const key: Key = { t, v, i: this.id('k') };
    if (options.legacy) {
      const [eo, ei] = LEGACY_EASES[options.legacy]!;
      key.eo = [...eo]; key.ei = [...ei]; key.hold = false;
    }
    if (options.hold) {
      if (options.legacy) key.hold = true;
      else Object.assign(key, linearEase(), { outInterp: 'hold' });
    }
    if (options.ease) Object.assign(key, linearEase(), options.ease);
    if (!options.legacy && !options.hold && !options.ease) Object.assign(key, linearEase());
    if (options.spring) key.spring = { damping: 18, stiffness: 180, mass: 1, ...options.spring };
    return key;
  }

  /** A full layer with `mkLayer` defaults for its type, then the given overrides. */
  layer(type: string, options: LayerOptions = {}): Layer {
    const comp = this.comp;
    const meta = typeMeta[type];
    if (!meta) throw new Error(`Unknown layer type ${type}`);
    const transformable = type !== 'audio';
    const layer: Layer = {
      id: options.id ?? this.id('L'),
      type,
      name: options.name ?? meta.label,
      from: options.from ?? 0,
      dur: options.dur ?? comp.dur,
      on: true, lock: false, shy: false, collapsed: true,
      threeD: false, solo: false,
      color: meta.color, blend: 'normal', mblur: false, parent: null, group: null,
      matteSource: null,
      p: transformable ? this.transform(type) : {},
      fx: [], transitionIn: null, transitionOut: null, masks: [],
      d: defaultContent(type, comp),
      locked_intent: {},
    };
    if (options.d) Object.assign(layer.d, options.d);
    for (const [key, value] of Object.entries(options.p ?? {})) {
      if (!layer.p[key]) throw new Error(`${type} layer has no channel ${key}`);
      layer.p[key] = typeof value === 'object' ? value : this.ch(value);
    }
    for (const field of ['blend', 'mblur', 'threeD', 'solo', 'parent', 'group', 'matteSource', 'on', 'lock', 'shy', 'fx', 'masks', 'transitionIn', 'transitionOut'] as const) {
      if (options[field] !== undefined) layer[field] = options[field];
    }
    // The app stores the matte mode as a channel (`editing.ts` writes `PM.P('alpha')`).
    if (options.matteMode !== undefined) layer.matteMode = typeof options.matteMode === 'string' ? this.ch(options.matteMode) : options.matteMode;
    return layer;
  }

  private transform(type: string): Record<string, Channel> {
    const comp = this.comp;
    const centered = !['group', 'solid', 'adjustment', 'shader', 'extension', 'precomp'].includes(type);
    const anchorAtOrigin = ['solid', 'adjustment', 'shader', 'extension', 'precomp'].includes(type);
    const values: Record<string, number> = {
      ...CHANNELS_3D,
      'anchor.x': type === 'null' ? 50 : 0, 'anchor.y': type === 'null' ? 50 : 0,
      'position.x': centered ? comp.w / 2 : 0, 'position.y': centered ? comp.h / 2 : 0,
      'scale.x': 100, 'scale.y': 100, rotation: 0, opacity: type === 'null' ? 0 : 100, skew: 0,
    };
    if (anchorAtOrigin) { values['anchor.x'] = 0; values['anchor.y'] = 0; }
    return Object.fromEntries(Object.entries(values).map(([key, value]) => [key, this.ch(value)]));
  }

  /** Adds a layer above everything added so far (index 0 renders on top, as `PM.addLayer` at index 0). */
  add(layer: Layer, comp: Doc = this.comp): Layer {
    comp.layers.unshift(layer);
    return layer;
  }

  effect(type: string, values: Record<string, Value | Channel> = {}, on: boolean | Channel = true): Doc {
    const definition = effectDef(type);
    const p: Record<string, Channel> = {};
    for (const param of definition.params) p[param.k] = this.ch(param.def as Value);
    for (const [key, value] of Object.entries(values)) {
      if (!p[key]) throw new Error(`Effect ${type} has no parameter ${key}`);
      p[key] = typeof value === 'object' ? value : this.ch(value);
    }
    return { id: this.id('f'), type, on, p };
  }

  transition(type: string, dur: number, values: Record<string, Value | Channel> = {}): Doc {
    const definition = transitionDef(type);
    const p: Record<string, Channel> = {};
    for (const param of definition.params) p[param.k] = this.ch(param.def as Value);
    for (const [key, value] of Object.entries(values)) {
      if (!p[key]) throw new Error(`Transition ${type} has no parameter ${key}`);
      p[key] = typeof value === 'object' ? value : this.ch(value);
    }
    return { type, dur, p };
  }

  mask(shape: 'rect' | 'ellipse', values: Partial<Record<'x' | 'y' | 'w' | 'h' | 'rotation' | 'feather', number | Channel>> = {},
    options: { mode?: 'add' | 'subtract' | Channel; on?: boolean | Channel; path?: Doc } = {}): Doc {
    const size = Math.round(Math.min(this.comp.w, this.comp.h) * .5);
    const defaults = { x: 0, y: 0, w: size, h: size, rotation: 0, feather: 24, ...values };
    return {
      id: this.id('K'), shape, mode: options.mode ?? 'add', on: options.on ?? true,
      ...(options.path ? { path: options.path } : {}),
      p: Object.fromEntries(Object.entries(defaults).map(([k, v]) => [k, typeof v === 'object' ? v : this.ch(v)])),
    };
  }

  /** An editable vector path (`makeVectorPath` defaults). */
  path(name: string, vertices: Array<[number, number, ...number[]]>, values: Record<string, Value | Channel> = {}): Doc {
    const defaults: Record<string, Value> = {
      x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, closed: false, fill: '#E8E2CF', fillEnabled: true,
      fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2, strokeOpacity: 100, trimStart: 0, trimEnd: 100,
      trimOffset: 0, copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0,
    };
    const p: Record<string, Channel> = {};
    for (const [key, value] of Object.entries(defaults)) p[key] = this.ch(value);
    for (const [key, value] of Object.entries(values)) {
      if (!p[key]) throw new Error(`Vector paths have no channel ${key}`);
      p[key] = typeof value === 'object' ? value : this.ch(value);
    }
    return {
      id: this.id('path'), name, parent: null, p,
      vertices: vertices.map(([x, y, inX = 0, inY = 0, outX = 0, outY = 0]) => ({
        id: this.id('v'),
        p: { x: this.ch(x), y: this.ch(y), inX: this.ch(inX), inY: this.ch(inY), outX: this.ch(outX), outY: this.ch(outY) },
      })),
    };
  }

  animator(preset: string, size: number, name = preset): Doc {
    return makeAnimator(v => this.ch(v as Value), this.id('ta'), name, findAnimatorPreset(preset), size);
  }

  /** A nested composition registered in the root's `comps`. */
  composition(options: CompOptions): Doc {
    const id = this.id('C');
    const comp = makeComposition(id, options);
    delete comp.compId; delete comp.compName; delete comp.comps; delete comp.exportDefaults;
    this.root.comps[id] = comp;
    return comp;
  }

  /** Runs `body` with layers defaulting against `comp`. */
  within<T>(comp: Doc, body: () => T): T {
    const previous = this.comp;
    this.comp = comp;
    try { return body(); } finally { this.comp = previous; }
  }

  captions(cues: Array<[number, number, string, boolean?]>, style: Record<string, unknown> = {}, preset = 'classic'): Doc {
    const base = captionModel.presetStyle(preset, this.comp.h);
    return captionModel.normalizeCaptionsContent({
      cues: cues.map(([start, end, text, words]) => ({
        id: this.id('c'), start, end, text,
        ...(words ? { words: wordTimings(text, start, end) } : {}),
      })),
      style: { ...base, ...style },
    });
  }

  object3d(object: Doc, options: LayerOptions & { position?: [number, number, number]; rotation?: [number, number, number]; scale?: [number, number, number] } = {}): Layer {
    return this.layer3d('object', { object }, options, [0, 0, 0]);
  }

  light3d(light: Doc, options: LayerOptions & { position?: [number, number, number] } = {}): Layer {
    return this.layer3d('light', { light }, options, [-3, 5, 4]);
  }

  camera3d(data: Doc, options: LayerOptions & { position?: [number, number, number] } = {}): Layer {
    return this.layer3d('camera', data, options, [3, 2, 6]);
  }

  private layer3d(role: 'object' | 'light' | 'camera', raw: Doc, options: LayerOptions & { position?: [number, number, number]; rotation?: [number, number, number]; scale?: [number, number, number] },
    fallback: [number, number, number]): Layer {
    const data = scene3d.parseLayer3DData(role, raw);
    const names = { object: '3D Model', light: 'Light', camera: 'Camera' } as const;
    const layer = this.layer('extension', { ...options, name: options.name ?? names[role] });
    layer.color = typeMeta.extension!.color;
    layer.d = { definition: scene3d.definitions[role], version: 1, w: this.comp.w, h: this.comp.h, data, params: {} };
    layer.threeD = true;
    const [x, y, z] = options.position ?? fallback;
    const [rx, ry, rz] = options.rotation ?? [0, 0, 0];
    const [sx, sy, sz] = options.scale ?? [100, 100, 100];
    const set = (path: string, value: number) => { if (!(options.p && path in options.p)) layer.p[path] = this.ch(value); };
    set('position.x', x); set('position.y', y); set('position.z', z);
    set('rotation.x', rx); set('rotation.y', ry); set('rotation', rz);
    set('scale.x', sx); set('scale.y', sy); set('scale.z', sz);
    return layer;
  }
}

export interface LayerOptions {
  id?: string; name?: string; from?: number; dur?: number;
  d?: Doc; p?: Record<string, Value | Channel>;
  blend?: string | Channel; mblur?: boolean | Channel; threeD?: boolean; solo?: boolean;
  parent?: string | null; group?: string | null; matteSource?: string | null; matteMode?: string | Channel;
  on?: boolean | Channel; lock?: boolean; shy?: boolean;
  fx?: Doc[]; masks?: Doc[]; transitionIn?: Doc | null; transitionOut?: Doc | null;
}

export function linearEase(): Record<string, unknown> {
  return {
    inInterp: 'linear', outInterp: 'linear',
    inEase: { speed: 0, influence: 100 / 3 }, outEase: { speed: 0, influence: 100 / 3 },
    autoBezier: false, continuous: false,
  };
}

function findAnimatorPreset(id: string): unknown {
  const preset = animatorPresets.find(item => item.id === id);
  if (!preset) throw new Error(`Unknown text animator preset ${id}`);
  return preset;
}

function wordTimings(text: string, start: number, end: number): Array<{ text: string; start: number; end: number }> {
  const words = text.split(/\s+/).filter(Boolean);
  const step = (end - start) / words.length;
  return words.map((word, index) => ({
    text: word,
    start: Math.round((start + index * step) * 1e6) / 1e6,
    end: Math.round((start + (index + 1) * step) * 1e6) / 1e6,
  }));
}

function defaultContent(type: string, comp: Doc): Doc {
  switch (type) {
    case 'solid': return { color: '#1b1b1f', w: comp.w, h: comp.h, radius: 0 };
    case 'shape': return { shape: 'rect', color: '#E8E2CF', w: 480, h: 480, radius: 24, stroke: 0, strokeColor: '#ffffff', points: 5 };
    case 'text': return {
      text: 'Powermove', boxWidth: { v: 0, kf: [], expr: null }, boxHeight: { v: 0, kf: [], expr: null },
      font: 'SF Pro Display', weight: 600, size: 128, tracking: -2, leading: 1.1, color: '#F2F2F2', align: 'center', italic: false,
    };
    case 'image': return { asset: null, fit: 'cover', w: 1920, h: 1080 };
    case 'video': return { asset: null, fit: 'cover', trim: 0, speed: 1, embeddedAudio: false, w: 1920, h: 1080 };
    case 'audio': return { asset: null, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 };
    case 'adjustment': return {};
    case 'shader': return { code: '', w: comp.w, h: comp.h, uniforms: {} };
    case 'extension': return { definition: '', version: 1, w: comp.w, h: comp.h, params: {}, data: {} };
    case 'null': return { color: '#6A6A70', w: 100, h: 100, radius: 0 };
    case 'group': return {};
    case 'precomp': return { comp: null, w: comp.w, h: comp.h };
    case 'captions': return { cues: [], style: captionModel.presetStyle('classic', comp.h) };
    default: throw new Error(`No content defaults for ${type}`);
  }
}

export function makeComposition(id: string, options: CompOptions): Doc {
  const w = options.w ?? 640, h = options.h ?? 360, fps = options.fps ?? 30, dur = options.dur ?? 3;
  const bg = options.bg ?? '#101014';
  const fill = options.backgroundFill ?? { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: bg.toUpperCase(), position: 0 }] };
  return {
    id, name: options.name, w, h, fps, dur, bg: fill.stops[0].color, backgroundFill: fill,
    compId: `${id}-root`, compName: options.name,
    layers: [], comps: {}, assets: {}, markers: [], work: [0, dur], params: {},
    revision: 0, edits: [], created: CREATED, shutter: options.shutter ?? .5,
  };
}

export function newProject(slug: string, options: CompOptions): Builder {
  const id = 'P' + slug.replace(/[^a-z0-9]+/gi, '-');
  return new Builder(slug, makeComposition(id, options));
}
