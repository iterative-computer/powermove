import type { Channel } from './project';

export type Value<T extends number | string | boolean> = T | Channel<T>;
/** Normalized object coordinates -> paint coordinates. Independent of raster density. */
export interface GradientPaint {
  type: Value<'linear' | 'radial' | 'angular' | 'diamond'>;
  transform: [Value<number>, Value<number>, Value<number>, Value<number>, Value<number>, Value<number>];
  stops: Array<{ position: Value<number>; color: Value<string> }>;
  opacity: Value<number>;
}
export interface ShapeStroke {
  align?: Value<'center' | 'inside' | 'outside'>;
  dashes?: Value<number>[];
  dashOffset?: Value<number>;
  cap?: Value<string>;
  startCap?: Value<string>;
  endCap?: Value<string>;
  join?: Value<'miter' | 'round' | 'bevel'>;
  miterLimit?: Value<number>;
  /** top, right, bottom, left; rects only */
  weights?: [Value<number>, Value<number>, Value<number>, Value<number>];
}
export interface ImageMapping {
  mode: Value<'fill' | 'fit' | 'crop' | 'tile'>;
  transform?: [Value<number>, Value<number>, Value<number>, Value<number>, Value<number>, Value<number>];
  scale?: Value<number>;
  rotation?: Value<number>;
}
