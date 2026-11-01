export { jsx, jsxs, Fragment } from "preact/jsx-runtime";
import type { ComponentChildren, VNode, JSX as PreactJSX } from "preact";
import type { Primitive, Json, SurfaceEvent } from "./protocol";
export interface SurfaceProps {
  children?: ComponentChildren;
  key?: string | number;
  id?: string;
  class?: string;
  role?: string;
  label?: string;
  labelledBy?: string;
  attr?: Record<string, Json>;
  state?: Record<string, Json>;
  style?: Record<string, string | number | null | { token: string }>;
  value?: Json;
  binding?: string;
  disabled?: boolean;
  checked?: boolean;
  tabIndex?: number;
  focusScope?: boolean;
  preventDefault?: Record<string, boolean | string[]>;
  onClick?: (event: SurfaceEvent) => void;
  onClickCapture?: (event: SurfaceEvent) => void;
  onInput?: (event: SurfaceEvent) => void;
  onKeyDown?: (event: SurfaceEvent) => void;
  onFocus?: (event: SurfaceEvent) => void;
  onBlur?: (event: SurfaceEvent) => void;
  onPointerDown?: (event: SurfaceEvent) => void;
  [unknownProp: string]: unknown;
}
export namespace JSX {
  export type Element = PreactJSX.Element;
  export type ElementType = Primitive | Exclude<PreactJSX.ElementType, string>;
  export type ElementChildrenAttribute = { children: unknown };
  export type IntrinsicElements = { [K in Primitive]: SurfaceProps };
}
