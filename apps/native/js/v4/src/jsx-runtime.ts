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
  title?: string;
  metadata?: Record<string, Json>;
  control?: string;
  mixed?: boolean;
  options?: Json[];
  selected?: string;
  open?: boolean;
  focusRequest?: string;
  "aria-checked"?: boolean | "mixed";
  "aria-haspopup"?: string;
  "aria-expanded"?: boolean;
  "aria-controls"?: string;
  "aria-selected"?: boolean;
  "aria-pressed"?: boolean;
  "aria-modal"?: boolean;
  "aria-valuenow"?: number;
  "data-pm-component"?: string;
  "data-pm-slot"?: string;
  /** How the native host presents this element: a borderless bar pop-up
   * (inline, or quiet: no wash), a row whose selection spans its whole band,
   * or code (a command, path or tool output, in the monospaced face). */
  "data-pm-variant"?: "inline" | "quiet" | "band" | "code";
  /** Files dropped from Finder land on this target. */
  "data-pm-drop"?: "files";
  /** An end-anchored scroller (`metadata.scrollAnchor: "end"`) reports
   * `{away}` when the reader leaves its end or returns. */
  onScroll?: (event: SurfaceEvent) => void;
  "data-pm-system-placement"?: "system";
  "data-pm-dismiss-outside"?: boolean;
  "data-pm-live-edit"?: boolean;
  "data-pm-indeterminate"?: boolean;
  onKeyUp?: (event: SurfaceEvent) => void;
  onPointerMove?: (event: SurfaceEvent) => void;
  onPointerEnter?: (event: SurfaceEvent) => void;
  onPointerLeave?: (event: SurfaceEvent) => void;
  onPointerUp?: (event: SurfaceEvent) => void;
  onPointerCancel?: (event: SurfaceEvent) => void;
  onDoubleClick?: (event: SurfaceEvent) => void;
  onCompositionStart?: (event: SurfaceEvent) => void;
  onCompositionUpdate?: (event: SurfaceEvent) => void;
  onCompositionEnd?: (event: SurfaceEvent) => void;
  onSelect?: (event: SurfaceEvent) => void;
  onChange?: (event: SurfaceEvent) => void;
  onDragStart?: (event: SurfaceEvent) => void;
  onDrag?: (event: SurfaceEvent) => void;
  onDragEnd?: (event: SurfaceEvent) => void;
  onDragEnter?: (event: SurfaceEvent) => void;
  onDragLeave?: (event: SurfaceEvent) => void;
  onDragOver?: (event: SurfaceEvent) => void;
  onDrop?: (event: SurfaceEvent) => void;
  onWheel?: (event: SurfaceEvent) => void;
  onRangeChange?: (event: SurfaceEvent) => void;
}
export namespace JSX {
  export type Element = PreactJSX.Element;
  export type ElementType = Primitive | Exclude<PreactJSX.ElementType, string>;
  export type ElementChildrenAttribute = { children: unknown };
  export type IntrinsicElements = {
    [K in Primitive]: {
      [P in keyof import("./schemas").PrimitiveProps[K]]:
        | import("./schemas").PrimitiveProps[K][P]
        | null;
    };
  };
}
