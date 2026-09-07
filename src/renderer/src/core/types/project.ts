import type { ExportDefaults } from '../export-defaults';

/**
 * Serializable Powermove project source.
 *
 * Time values are seconds. Layer property channels use the legacy flat,
 * dot-separated names (for example `position.x`).
 */

export const TYPE_META = {
  solid: { icon: 'grid', color: '#4C8DFF', label: 'Solid' },
  text: { icon: 'code', color: '#E8E2CF', label: 'Text' },
  shape: { icon: 'grid', color: '#6C7BE8', label: 'Shape' },
  image: { icon: 'layers', color: '#A9A9AE', label: 'Image' },
  video: { icon: 'cam', color: '#3B62E8', label: 'Video' },
  audio: {
    icon: 'clock', color: '#4C8DFF', label: 'Audio', visual: false,
    transform: false, effects: false, masks: false, pickable: false
  },
  adjustment: { icon: 'wand', color: '#A78BFA', label: 'Adjustment', pickable: false },
  shader: { icon: 'wand', color: '#FF6B1A', label: 'Shader' },
  extension: { icon: 'layers', color: '#9B8CFF', label: 'Extension' },
  null: { icon: 'dot', color: '#6a6a70', label: 'Null', visual: false, pickable: false },
  group: { icon: 'layers', color: '#3FCF8E', label: 'Group', visual: false, transform: true, effects: false, masks: false, pickable: false },
  precomp: { icon: 'layers', color: '#3FCF8E', label: 'Precomp' }
} as const;

export type LayerType = keyof typeof TYPE_META;
export type LayerTypeMeta = (typeof TYPE_META)[LayerType];
export type LayerTypeIcon = LayerTypeMeta['icon'];
export type LayerTypeColor = LayerTypeMeta['color'];
export type LayerTypeLabel = LayerTypeMeta['label'];
export type NonVisualLayerType = {
  [T in LayerType]: (typeof TYPE_META)[T] extends { readonly visual: false } ? T : never;
}[LayerType];
export type NonTransformableLayerType = {
  [T in LayerType]: (typeof TYPE_META)[T] extends { readonly transform: false } ? T : never;
}[LayerType];

export const BLEND_MODES = [
  'normal', 'add', 'screen', 'multiply', 'overlay', 'softlight', 'difference', 'lighten', 'darken'
] as const;
export type BlendMode = (typeof BLEND_MODES)[number];

export type ChannelValue = number | string | boolean;
export type EaseHandle = [number, number];

export type { InterpolationType, TemporalEase } from '../anim/temporal-ease';

export interface Keyframe<T extends ChannelValue = ChannelValue> {
  t: number;
  v: T;
  /** After Effects temporal interpolation, per side of the key. */
  inInterp: import('../anim/temporal-ease').InterpolationType;
  outInterp: import('../anim/temporal-ease').InterpolationType;
  /** Speed in value units per second, influence as a percent of the segment. */
  inEase: import('../anim/temporal-ease').TemporalEase;
  outEase: import('../anim/temporal-ease').TemporalEase;
  /** AE "Auto Bezier": the tangent follows the neighbouring keys. */
  autoBezier: boolean;
  /** AE "Continuous Bezier": one speed carries through the key. */
  continuous: boolean;
  i: string;
}

export interface Channel<T extends ChannelValue = ChannelValue> {
  v: T;
  kf: Array<Keyframe<T>>;
  expr: string | null;
}

export type TransformChannelName =
  | 'position.z' | 'anchor.z' | 'scale.z' | 'rotation.x' | 'rotation.y'
  | 'perspective'
  | 'orientation.x' | 'orientation.y' | 'orientation.z'
  | 'anchor.x'
  | 'anchor.y'
  | 'position.x'
  | 'position.y'
  | 'scale.x'
  | 'scale.y'
  | 'rotation'
  | 'opacity'
  | 'skew';

export type TransformChannels = Record<TransformChannelName, Channel<number>>;

export interface Mask {
  path?: VectorPath;
  id: string;
  shape: 'rect' | 'ellipse';
  mode: 'add' | 'subtract';
  on: boolean;
  p: {
    x: Channel<number>;
    y: Channel<number>;
    w: Channel<number>;
    h: Channel<number>;
    rotation: Channel<number>;
    feather: Channel<number>;
  };
}

export interface Effect {
  id: string;
  type: string;
  on: boolean;
  /** Added when an effect is inserted through Edit.add_effect. */
  open?: boolean;
  p: Record<string, Channel>;
}

export interface Transition {
  type: string;
  dur: number;
  p: Record<string, Channel>;
  /** hydrate() marks transitions whose extension definition is unavailable. */
  missing?: boolean;
}

/** Editable paths and text controls use the same channels as transforms. */
export interface PathVertex { id:string; p:Record<'x'|'y'|'inX'|'inY'|'outX'|'outY',Channel<number>> }
export interface VectorPath { id:string; name:string; parent:string|null; vertices:PathVertex[]; p:Record<string,Channel> }
export interface TextAnimator { id:string; name:string; p:Record<string,Channel> }
export interface TextStyleRange { id:string; start:number; end:number; p:Record<string,Channel> }

export interface SolidContent {
  [key: string]: unknown;
  color: string;
  w: number;
  h: number;
  radius: number;
}

export interface TextContent {
  animators?: TextAnimator[];
  styles?: TextStyleRange[];
  [key: string]: unknown;
  text: string;
  /** Zero width is point text. Positive values are an animatable paragraph box. */
  boxWidth: Channel<number>;
  boxHeight: Channel<number>;
  font: string;
  weight: number;
  size: number;
  tracking: number;
  leading: number;
  color: string;
  align: 'left' | 'center' | 'right';
  italic: boolean;
}

export interface ShapeContent {
  paths?: VectorPath[];
  [key: string]: unknown;
  shape: 'rect' | 'ellipse' | 'polygon' | 'star' | 'line';
  color: string;
  w: number;
  h: number;
  radius: number;
  stroke: number;
  strokeColor: string;
  points: number;
}

export interface ImageContent {
  [key: string]: unknown;
  asset: string | null;
  fit: 'cover' | 'contain' | 'stretch';
  w: number;
  h: number;
}

export interface VideoContent extends ImageContent {
  trim: number;
  speed: number;
}

export interface AudioContent {
  asset: string | null;
  gain: number;
  trim: number;
  fadeIn: number;
  fadeOut: number;
}

/**
 * An adjustment layer has no pixels of its own. Its effects process the
 * accumulated composition beneath it, while ordinary layer timing, opacity,
 * masks, keyframes, and Undo remain canonical project source.
 */
export interface AdjustmentContent {
  [key: string]: unknown;
}

export interface ShaderContent {
  [key: string]: unknown;
  code: string;
  w: number;
  h: number;
  uniforms: Record<string, Channel>;
}

/**
 * Structured instance data for a layer whose rendering capability is supplied
 * by an extension. The definition's code stays in the extension; the project
 * stores only editable values and opaque JSON data. `params` are ordinary
 * Powermove channels, so they participate in keyframes, expressions and Undo.
 */
export interface ExtensionLayerContent {
  [key: string]: unknown;
  definition: string;
  version: number;
  w: number;
  h: number;
  params: Record<string, Channel>;
  data: Record<string, ProjectJsonValue>;
}

export interface NullContent {
  [key: string]: unknown;
}

export interface PrecompContent {
  trim?: Channel<number>;
  timeRemap?: Channel<boolean>;
  sourceTime?: Channel<number>;
  [key: string]: unknown;
  comp: string | null;
  w: number;
  h: number;
}

export interface LockedIntentEntry {
  by: string;
  at: number;
  t: number;
}

interface LayerBase<T extends LayerType, D extends object> {
  id: string;
  type: T;
  name: string;
  from: number;
  dur: number;
  on: boolean;
  lock: boolean;
  shy: boolean;
  matteSource?: string | null;
  matteMode?: Channel<'alpha'|'alpha-inverted'|'luma'|'luma-inverted'>;
  solo?: boolean;
  collapsed: boolean;
  scaleLinked?: boolean;
  threeD?: boolean;
  color: string;
  blend: BlendMode;
  mblur: boolean;
  group?: string | null;
  parent: string | null;
  p: T extends 'audio' ? Record<never, never> : TransformChannels;
  fx: Effect[];
  transitionIn?: Transition | null;
  transitionOut?: Transition | null;
  masks: Mask[];
  d: D;
  locked_intent: Record<string, LockedIntentEntry | Record<string, unknown>>;
}

export type SolidLayer = LayerBase<'solid', SolidContent>;
export type TextLayer = LayerBase<'text', TextContent>;
export type ShapeLayer = LayerBase<'shape', ShapeContent>;
export type ImageLayer = LayerBase<'image', ImageContent>;
export type VideoLayer = LayerBase<'video', VideoContent>;
export type AudioLayer = LayerBase<'audio', AudioContent>;
export type AdjustmentLayer = LayerBase<'adjustment', AdjustmentContent>;
export type ShaderLayer = LayerBase<'shader', ShaderContent>;
export type ExtensionLayer = LayerBase<'extension', ExtensionLayerContent>;
export type NullLayer = LayerBase<'null', NullContent>;
export type PrecompLayer = LayerBase<'precomp', PrecompContent>;

/** `type` and `d` narrow together, matching mkLayer's DEFAULTS table. */
export type Layer =
  | SolidLayer
  | TextLayer
  | ShapeLayer
  | ImageLayer
  | VideoLayer
  | AudioLayer
  | AdjustmentLayer
  | ShaderLayer
  | ExtensionLayer
  | NullLayer
  | LayerBase<'group', NullContent>
  | PrecompLayer;

export interface FillStop {
  id: string;
  color: string;
  position: number;
}

export interface Fill {
  type: 'solid' | 'linear' | 'radial' | 'none';
  angle: number;
  stops: FillStop[];
}

export interface SceneSelectOption {
  v: unknown;
  label?: string;
  [key: string]: unknown;
}

interface SceneParamBase {
  name: string;
  label: string;
  [key: string]: unknown;
}

export interface NumberSceneParam extends SceneParamBase {
  control: 'num';
  value: number;
  min?: number;
  max?: number;
}

export interface ColorSceneParam extends SceneParamBase {
  control: 'color';
  value: string;
}

export interface ToggleSceneParam extends SceneParamBase {
  control: 'toggle';
  value: boolean;
}

export interface SelectSceneParam extends SceneParamBase {
  control: 'select';
  value: unknown;
  options: SceneSelectOption[];
}

export type SceneParam = NumberSceneParam | ColorSceneParam | ToggleSceneParam | SelectSceneParam;

export interface Marker {
  /** Legacy demo markers predate marker IDs, while Edit.add_marker always adds one. */
  id?: string;
  t: number;
  name: string;
}

export type ProjectJsonPrimitive = string | number | boolean | null;
export type ProjectJsonValue =
  | ProjectJsonPrimitive
  | ProjectJsonValue[]
  | { [key: string]: ProjectJsonValue };
export type EditLogOperation = { [key: string]: ProjectJsonValue };

export interface EditLogEntry {
  id: string;
  revision: number;
  at: number;
  origin: string;
  label: string;
  summary: string[];
  operations: EditLogOperation[];
}

/**
 * A nested composition is created with mkProject too. It consequently carries
 * the same serializable fields as the root project, including its own `comps`
 * map, even though the current UI only creates one nesting level at a time.
 */
export interface Comp {
  id: string;
  name: string;
  w: number;
  h: number;
  fps: number;
  dur: number;
  bg: string;
  backgroundFill: Fill;
  layers: Layer[];
  comps: Record<string, Comp>;
  /** Asset metadata is owned by the legacy media subsystem and is free-form. */
  assets: Record<string, unknown>;
  markers: Marker[];
  work: [number, number];
  params: Record<string, SceneParam>;
  revision: number;
  edits: EditLogEntry[];
  created: number;
  /** hydrate() supplies this migration default; mkProject itself does not. */
  shutter?: number;
  /** Per-project export settings; nested comps carry the field but never use it. */
  exportDefaults?: ExportDefaults;
  /** The legacy library module installs this lazily. */
  library?: Record<string, unknown>;
  /** Free-form project notes are persisted by the demo and user projects. */
  notes?: string;
}

export interface Project extends Comp {}
