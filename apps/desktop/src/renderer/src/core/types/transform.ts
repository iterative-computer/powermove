import type { LayerType } from './project';

/** Literal vocabulary accepted by `PM.Capabilities.sanitizeTransform`. */
export const TRANSFORM_SCOPES = ['selection', 'all', 'visible'] as const;
export const TRANSFORM_ORDERS = [
  'stack',
  'reverseStack',
  'selection',
  'reverseSelection',
  'start',
  'reverseStart',
  'name',
  'random'
] as const;
export const TRANSFORM_OPS = [
  'add',
  'subtract',
  'multiply',
  'divide',
  'min',
  'max',
  'clamp',
  'round',
  'floor',
  'ceil',
  'abs',
  'negate',
  'frames',
  'equal',
  'if'
] as const;
export const TRANSFORM_REFS = [
  'current',
  'index',
  'count',
  'fps',
  'playhead',
  'composition.duration'
] as const;
export const TRANSFORM_AGGREGATES = ['min', 'max', 'first', 'last', 'sum', 'average'] as const;

export const SCOPES = TRANSFORM_SCOPES;
export const ORDERS = TRANSFORM_ORDERS;
export const OPS = TRANSFORM_OPS;
export const REFS = TRANSFORM_REFS;
export const AGGREGATES = TRANSFORM_AGGREGATES;

export type TransformScope = (typeof TRANSFORM_SCOPES)[number];
export type TransformOrder = (typeof TRANSFORM_ORDERS)[number];
export type TransformOp = (typeof TRANSFORM_OPS)[number];
export type TransformRef = (typeof TRANSFORM_REFS)[number];
export type TransformAggregate = (typeof TRANSFORM_AGGREGATES)[number];
export type TransformOrderName = TransformOrder;
export type TransformOperator = TransformOp;
export type TransformReference = TransformRef;
export type TransformAggregateKind = TransformAggregate;

/* Short aliases keep the grammar readable at call sites. */
export type Scope = TransformScope;
export type Order = TransformOrder;
export type Op = TransformOp;
export type Ref = TransformRef;
export type Aggregate = TransformAggregate;

export const LAYER_TRANSFORM_FIELDS = [
  'name',
  'from',
  'duration',
  'visible',
  'locked',
  'shy',
  'blend',
  'motionBlur',
  'parent',
  'color',
  'collapsed'
] as const;

export type LayerTransformField = (typeof LAYER_TRANSFORM_FIELDS)[number];
export type TransformPath =
  | `layer.${LayerTransformField}`
  | `properties.${string}`
  | `content.${string}`;

export interface Selector {
  scope: TransformScope;
  /** Always false after sanitization: generated transforms cannot bypass locks. */
  includeLocked: false;
  types: LayerType[];
}

export type TransformPrimitive = null | string | boolean | number;

export interface TransformStateExpression {
  state: string;
  fallback?: TransformPrimitive;
}

export interface TransformRefExpression {
  ref: TransformRef;
}

export interface TransformAggregateExpression {
  aggregate: TransformAggregate;
  path: TransformPath;
}

type UnaryTransformArgs = [TransformExpression];
type BinaryTransformArgs = [TransformExpression, TransformExpression];
type TernaryTransformArgs = [TransformExpression, TransformExpression, TransformExpression];
export type OneToEightTransformArgs = [
  TransformExpression,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?
];
export type TwoToEightTransformArgs = [
  TransformExpression,
  TransformExpression,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?,
  TransformExpression?
];

export type TransformOperationExpression =
  | { op: 'add' | 'multiply'; args: TwoToEightTransformArgs }
  | { op: 'subtract' | 'divide' | 'equal'; args: BinaryTransformArgs }
  | { op: 'min' | 'max'; args: OneToEightTransformArgs }
  | { op: 'clamp' | 'if'; args: TernaryTransformArgs }
  | { op: 'round' | 'floor' | 'ceil' | 'abs' | 'negate' | 'frames'; args: UnaryTransformArgs };

/**
 * The bounded, data-only expression tree accepted by Capabilities. Arity and
 * depth/node limits are runtime constraints; this union captures its shape.
 */
export type TransformExpression =
  | TransformPrimitive
  | TransformStateExpression
  | TransformRefExpression
  | TransformAggregateExpression
  | TransformOperationExpression;

export interface TransformEdit {
  path: TransformPath;
  value: TransformExpression;
}

export interface TransformStateOrder {
  state: string;
  fallback: TransformOrder;
}

/** Ergonomic authored selector accepted before sanitization fills defaults. */
export interface SelectorInput {
  scope?: TransformScope;
  includeLocked?: false;
  types?: LayerType[];
}

export type TransformInputSelector = SelectorInput;
export type OrderInput = TransformOrder | Partial<TransformStateOrder>;

/**
 * A valid authored transform. `sanitizeTransform` accepts this abbreviated
 * shape and returns the canonical `Transform` below.
 */
export interface TransformInput {
  version?: 1;
  label?: string;
  selector?: SelectorInput;
  order?: OrderInput;
  seed?: string;
  edits: TransformEdit[];
}

/** Fully sanitized transform returned by `sanitizeTransform`. */
export interface Transform {
  version: 1;
  label: string;
  selector: Selector;
  order: TransformOrder | TransformStateOrder;
  seed: string;
  edits: TransformEdit[];
}
