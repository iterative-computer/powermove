import type { Json } from "./protocol";
import type { SurfaceProps } from "./jsx-runtime";

/** Explicit extension data. Native renderers never interpret this bag. */
export type Metadata = Record<string, Json>;
/** Opaque host-owned lease, scoped to realm generation. */
export type ResourceHandle = string;
export type Anchor = number | { x: number; y: number };
export interface Point {
  x: number;
  y: number;
}
export interface PrimitiveProps {
  div: SurfaceProps;
  text: SurfaceProps;
  img: SurfaceProps & {
    src?: ResourceHandle;
    resource?: ResourceHandle;
    alt?: string;
  };
  input: SurfaceProps & {
    value?: string | number;
    checked?: boolean;
    min?: number;
    max?: number;
    step?: number;
    speed?: number;
    precision?: number;
    unit?: string;
  };
  textarea: SurfaceProps & { value?: string };
  button: SurfaceProps;
  scroll: SurfaceProps;
  "virtual-list": SurfaceProps & {
    count?: number;
    range?: [number, number];
    itemSize?: number;
  };
  anchored: SurfaceProps & { anchor?: Anchor; placement?: string };
  path: SurfaceProps & { d?: string; points?: Point[] };
  canvas: SurfaceProps & {
    surface?: ResourceHandle;
    resource?: ResourceHandle;
  };
  frame: SurfaceProps & {
    surface?: ResourceHandle;
    resource?: ResourceHandle;
    time?: number;
  };
  markdown: SurfaceProps & { source?: string };
  code: SurfaceProps & { source?: string; language?: string };
  Glass: SurfaceProps & { surface?: string; material?: string };
  material: SurfaceProps & { material?: string };
  webview: SurfaceProps & { src?: string; source?: string };
}
