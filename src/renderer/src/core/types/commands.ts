import type { BlendMode, Layer, LayerType } from './project';
import type { TransformInput } from './transform';

export const COMMAND_TYPES = [
  'set_property',
  'replace_keyframes',
  'set_easing',
  'set_expression',
  'set_content',
  'set_layer',
  'set_composition',
  'add_layer',
  'delete_layers',
  'reorder_layer',
  'add_effect',
  'remove_effect',
  'set_effect',
  'set_scene_parameter',
  'add_marker',
  'create_section',
  'update_section',
  'transform_layers'
] as const;

export type EditCommandType = (typeof COMMAND_TYPES)[number];

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

/** A falsey/missing target means the current selection in the legacy editor. */
export type LayerTarget = null | '$selection' | 'selection' | number | string;

export type EasingCurve = readonly [number, number, number, number];
export type Easing = string | EasingCurve;

export interface CommandKeyframe {
  time: number;
  value: JsonValue;
  ease?: string;
  hold?: boolean;
}

interface LayerTargetedCommand {
  target?: LayerTarget;
}

interface LockableCommand {
  overrideLock?: boolean;
}

interface IntentAwareCommand {
  preserveHandEdits?: boolean;
}

export interface SetPropertyCommand extends LayerTargetedCommand, LockableCommand, IntentAwareCommand {
  type: 'set_property';
  path: string;
  value: JsonValue;
  time?: number;
  mode?: 'auto' | 'static' | 'keyframe';
  ease?: string;
  hold?: boolean;
  markIntent?: boolean | string;
}

export interface ReplaceKeyframesCommand extends LayerTargetedCommand, LockableCommand, IntentAwareCommand {
  type: 'replace_keyframes';
  path: string;
  keyframes: CommandKeyframe[];
  replace?: boolean;
  expression?: string | null;
}

export interface SetEasingCommand {
  type: 'set_easing';
  keyframes: string[];
  curve: Easing;
}

export interface SetExpressionCommand extends LayerTargetedCommand {
  type: 'set_expression';
  path: string;
  expression: string | null;
}

export interface SetContentCommand extends LayerTargetedCommand, LockableCommand {
  type: 'set_content';
  patch: JsonObject;
}

export interface LayerPatch {
  name?: string;
  from?: number;
  duration?: number;
  visible?: boolean;
  locked?: boolean;
  solo?: boolean;
  shy?: boolean;
  blend?: BlendMode;
  motionBlur?: boolean;
  parent?: LayerTarget;
  color?: string;
  collapsed?: boolean;
}

export interface SetLayerCommand extends LayerTargetedCommand {
  type: 'set_layer';
  patch: LayerPatch;
}

export interface CompositionPatch {
  name?: string;
  width?: number;
  height?: number;
  fps?: number;
  duration?: number;
  background?: string;
  /** normalizeFill resolves this input against the composition's current background. */
  backgroundFill?: JsonValue;
  shutter?: number;
  workArea?: [number, number];
}

export interface SetCompositionCommand {
  type: 'set_composition';
  patch: CompositionPatch;
}

export interface AddLayerCommand {
  type: 'add_layer';
  id?: string;
  layerType: LayerType;
  name?: string;
  from?: number;
  duration?: number;
  content?: JsonObject;
  properties?: JsonObject;
  color?: string;
  index?: number;
  select?: boolean;
  parent?: LayerTarget;
  blend?: BlendMode;
  motionBlur?: boolean;
  visible?: boolean;
  solo?: boolean;
  shy?: boolean;
  collapsed?: boolean;
}

export interface DeleteLayersCommand extends LayerTargetedCommand {
  type: 'delete_layers';
  targets?: LayerTarget[];
}

export interface ReorderLayerCommand extends LayerTargetedCommand {
  type: 'reorder_layer';
  index: number;
}

export interface AddEffectCommand extends LayerTargetedCommand {
  type: 'add_effect';
  effect: string;
  parameters?: JsonObject;
  open?: boolean;
}

export interface RemoveEffectCommand extends LayerTargetedCommand {
  type: 'remove_effect';
  effect: string;
}

export interface SetEffectCommand extends LayerTargetedCommand {
  type: 'set_effect';
  effect: string;
  patch: {
    enabled?: boolean;
    open?: boolean;
  };
}

export interface SetSceneParameterCommand {
  type: 'set_scene_parameter';
  name: string;
  label?: string;
  control?: string;
  value: JsonValue;
  min?: number;
  max?: number;
  options?: JsonValue[];
}

export interface AddMarkerCommand {
  type: 'add_marker';
  id?: string;
  time?: number;
  name?: string;
}

export interface SectionVersion {
  id: string;
  schemaVersion?: number;
  layers: Layer[];
  thumb?: string | null;
  at: number;
}

export interface SectionManifest {
  id: string;
  schemaVersion?: number;
  name?: string;
  at?: number;
  thumb?: string | null;
  sourceProjectId?: string;
  sourceProjectName?: string;
  tags?: string[];
  deletedAt?: number | null;
  comments?: JsonValue[];
  layers?: Layer[];
  versions?: SectionVersion[];
  [key: string]: JsonValue | Layer[] | SectionVersion[] | undefined;
}

export interface CreateSectionCommand {
  type: 'create_section';
  section: SectionManifest;
}

export interface UpdateSectionCommand {
  type: 'update_section';
  sectionId: string;
  layers: Layer[];
  thumb?: string | null;
  at?: number;
  version?: SectionVersion;
}

export interface TransformLayersCommand {
  type: 'transform_layers';
  transform: TransformInput;
  state?: Record<string, JsonValue>;
}

export type EditCommand =
  | SetPropertyCommand
  | ReplaceKeyframesCommand
  | SetEasingCommand
  | SetExpressionCommand
  | SetContentCommand
  | SetLayerCommand
  | SetCompositionCommand
  | AddLayerCommand
  | DeleteLayersCommand
  | ReorderLayerCommand
  | AddEffectCommand
  | RemoveEffectCommand
  | SetEffectCommand
  | SetSceneParameterCommand
  | AddMarkerCommand
  | CreateSectionCommand
  | UpdateSectionCommand
  | TransformLayersCommand;

export interface EditMeta {
  label?: string;
  origin?: string;
  baseRevision?: number;
  historyGroup?: string | null;
}

export interface AppliedEditResult {
  command: EditCommand;
  data: Record<string, unknown>;
}

export type EditResult =
  | { ok: false; message: string }
  | {
      ok: true;
      message: string;
      data: {
        results?: AppliedEditResult[];
        revision?: number;
        [key: string]: unknown;
      };
    };

export function assertNever(value: never, message = 'Unhandled edit command'): never {
  throw new Error(`${message}: ${String(value)}`);
}
