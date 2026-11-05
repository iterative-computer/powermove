import type { Modifiers } from "./model";
import type { Json } from "../../src/protocol";

/** Native hit-test/field actions. Times are composition seconds; drag samples
 * are totals from pointer-down. The Rust worker owns previews and history. */
export type NativeDrag =
  | {
      kind:
        | "layers"
        | "trimIn"
        | "trimOut"
        | "offsetLayers"
        | "keys"
        | "graphKeys";
      ids: string[];
    }
  | {
      kind: "stretchKeys";
      ids: string[];
      anchorTime: number;
      grabbedTime: number;
    }
  | {
      kind: "transformKeys";
      ids: string[];
      anchorTime: number;
      anchorValue: number;
    }
  | { kind: "handle"; id: string; side: "in" | "out" }
  | { kind: "property"; layer: string; path: string }
  | { kind: "workIn" | "workOut" | "workMove" };
export interface TrackTarget {
  group?: string | null;
  track: number;
}
export type ProEdit =
  | { operation: "move" | "slip"; ids: string[]; delta: number }
  | { operation: "trim"; ids: string[]; edge: "in" | "out"; delta: number }
  | { operation: "rippleDelete"; ids: string[]; tracks: TrackTarget[] }
  | {
      operation: "rippleTrim";
      layer: string;
      edge: "in" | "out";
      delta: number;
      tracks: TrackTarget[];
    }
  | { operation: "roll"; left: string; right: string; delta: number }
  | {
      operation: "slide";
      previous: string;
      layer: string;
      next: string;
      delta: number;
    };
export type NativeAction = (
  | ({ type: "beginDrag" } & NativeDrag)
  | {
      type: "drag";
      timeDelta?: number;
      valueDelta?: number;
      timeScale?: number;
      valueScale?: number;
      handleTime?: number;
      handleValue?: number;
      acquireTolerance?: number;
      releaseTolerance?: number;
      modifiers?: Modifiers;
    }
  | { type: "endDrag" | "cancelDrag" | "dispose" }
  | {
      type: "selectKeys";
      layer: string;
      path: string;
      ids: string[];
      additive?: boolean;
      dragging?: boolean;
    }
  | { type: "selectProperty"; layer: string; path: string }
  | { type: "parent"; ids: string[]; parent: string | null }
  | { type: "separateDimensions"; layer: string; separate: boolean }
  | { type: "editKeys"; ids: string[]; time?: number; value?: Json }
  | { type: "easing"; ids: string[]; curve: [number, number, number, number] }
  | { type: "easing"; ids: string[]; preset: string }
  | {
      type: "interpolation";
      ids: string[];
      side: "in" | "out" | "both";
      interpolation: "linear" | "bezier" | "hold" | "continuous" | "auto";
    }
  | { type: "simplify"; ids: string[] }
  | { type: "command"; command: string }
  | ({ type: "proEdit"; linked?: boolean } & ProEdit)
) & { document?: string; revision?: string };
