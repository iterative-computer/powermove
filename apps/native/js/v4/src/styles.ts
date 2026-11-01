import type { SurfaceProps } from "./jsx-runtime";
import type { Json } from "./protocol";
export type Selector =
  | { kind: "id" | "class" | "role"; value: string }
  | { kind: "attr" | "state"; value: { name: string; value: Json } }
  | { kind: "all"; value: Selector[] };
export interface StyleRule {
  selector: Selector;
  style: NonNullable<SurfaceProps["style"]>;
}
