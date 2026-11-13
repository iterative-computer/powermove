import type { EditCommand, HostOperation, HostQuery } from "../../src/host-api";
import type { Json } from "../../src/protocol";
import type { Option, SegmentOption } from "../../src/components";

/** Read-only presentation, supplied by pm-panel-inspector's application adapter.
 * No writable document, layer collection or 60k-selection expansion belongs here.
 */
export type Target =
  | { kind: "no-project" }
  | { kind: "composition"; key: string }
  | {
      kind: "layers";
      key: string;
      count: number;
      first: {
        id: string;
        name: string;
        type: string;
        typeLabel: string;
        locked: boolean;
        audio: boolean;
        acceptsEffects: boolean;
        /** A Studio 3D layer: 2D alignment doesn't apply to it. */
        studio?: boolean;
        /** The viewer couldn't draw this layer: what happened and the fix. */
        error?: string;
      };
      sharedType: string | null;
    };
export interface Binding {
  /** Model-issued, generation-owned handle. Never manufacture a path as a handle. */
  handle: string;
  channels: string[];
}
interface Field<T> {
  binding: Binding;
  value: T;
  mixed?: boolean;
  disabled?: boolean;
}
export interface NumberAxis extends Field<number> {
  axis?: string;
  unit?: string;
  step: number;
  precision?: number;
  min?: number;
  max?: number;
}
export type Control =
  | { kind: "number" | "point"; axes: NumberAxis[] }
  | ({ kind: "text"; multiline?: boolean } & Field<string>)
  | ({ kind: "choice" | "font"; options: (string | Option)[] } & Field<Json>)
  | ({ kind: "toggle" } & Field<boolean>)
  | ({ kind: "colour"; hexBinding?: Binding; hex?: string } & Field<string>)
  | ({ kind: "fill"; description: string } & Field<Json>)
  | ({ kind: "segmented"; options: SegmentOption[] } & Field<string>)
  /** A read-only fact in the value column (`Font  Not installed`). */
  | { kind: "status"; value: string };
export interface Row {
  id: string;
  label: string;
  control: Control;
  locked?: boolean;
  disabled?: boolean;
  /** Addressable by intents but drawn elsewhere (the finish shelf). */
  hidden?: boolean;
  animation?: "static" | "animated" | "key-at-playhead";
  linked?: boolean;
  expression?: { binding: Binding; source: string; error: string | null };
  error?: { kind: "number" } | { kind: "range"; min: number; max: number };
}
export type SectionAction =
  | "mask"
  | "animator"
  | "release-precomp"
  | "refresh-fonts"
  | "set-up-studio"
  | "time-remap"
  | "freeze-frame"
  | "reverse";
export interface AnimatorPreset {
  id: string | null;
  label: string;
  disabled: boolean;
}
/** A finish tile drawn as a tiny shaded sphere (colours are content). */
export interface Finish {
  id: string;
  label: string;
  rim: string;
  body: string;
  highlight: string;
  /** The lit body's size, the highlight's and the glint's, as percentages
   * of the tile (swatch artwork, from the material). */
  size: string;
  spot: string;
  glintSize: string;
  /** A clear coat's sharp glint colour, or null for none. */
  glint: string | null;
}
export interface Section {
  id: string;
  title: string;
  collapsed: boolean;
  rows: Row[];
  action?: SectionAction;
  /** Several quiet actions in a row (Time: remap, freeze, reverse). */
  actions?: SectionAction[];
  animators?: { id: string; name: string }[];
  presets?: AnimatorPreset[];
  /** Masks: a heading row with Remove, then that mask's rows. */
  masks?: { id: string; name: string; rows: Row[] }[];
  /** One quiet line under the rows that explains them (a 3D preset's cost). */
  note?: string;
  /** Studio finishes: swatch tiles under the heading; a click applies one. */
  finishes?: Finish[];
  /** The imported-model part a viewer double-click picked. */
  focused?: boolean;
}
export interface InspectorSnapshot {
  target: Target;
  /** Already ordered, applicable rows only. Mixed state computed by the model
   * for visible properties, never by walking selected layers in the view. */
  sections: Section[];
  playing: boolean;
  time: number;
}
export type Alignment =
  | "left"
  | "hcenter"
  | "right"
  | "top"
  | "vcenter"
  | "bottom";
export type RowAction =
  | "add-key"
  | "remove-animation"
  | "edit-expression"
  | "remove-expression"
  | "reset"
  | "show-graph"
  | "link";
export type Intent =
  | { type: "key"; row: string; removeAnimation: boolean }
  | { type: "row-action"; row: string; action: RowAction }
  | { type: "align"; alignment: Alignment }
  | { type: "distribute"; axis: "horizontal" | "vertical" }
  | { type: "collapse"; section: string; collapsed: boolean }
  | { type: "section-action"; section: string; action: SectionAction }
  | { type: "drop-effect"; effect: Json; target: string }
  | { type: "add-animator"; preset: string }
  | { type: "remove-animator"; animator: string }
  | { type: "remove-mask"; mask: string }
  | { type: "set-value"; row: string; value: Json };
export type ModelOperation = Extract<
  HostOperation,
  { type: "command" | "edit" }
>;
/** Real model owns applicability, ancestor locks, scale ratios, current time,
 * mixed deltas, animation/expressions and typed kernel command translation.
 * Null means declined; it must never mean a fabricated successful write.
 */
export interface InspectorModel {
  update(query: HostQuery, publication: Json): void;
  inspect(): InspectorSnapshot;
  plan(intent: Intent): ModelOperation | null;
}
export interface MenuItem {
  label: string;
  action: RowAction;
  enabled: boolean;
  checked?: boolean;
  separatorBefore?: boolean;
}
export type NativeRequest =
  | { type: "add-effect-menu"; anchor: number; target: string }
  | { type: "new-layer-menu"; anchor: number; items: (string | null)[] }
  | { type: "row-menu"; anchor: number; row: string; items: MenuItem[] }
  | { type: "animator-menu"; anchor: number; items: AnimatorPreset[] }
  | { type: "font-menu"; anchor: number; row: string }
  | { type: "fill-popover"; anchor: number; row: string }
  | { type: "point-pad"; anchor: number; row: string }
  | { type: "parent-pick"; anchor: number; row: string }
  | { type: "focus-timeline" };
/** Native controls/menu callbacks enqueue model intents; never synchronously call
 * JS on the UI thread. Implemented by a Linux recorder in tests, AppKit in app.
 */
export interface NativeServices {
  request(request: NativeRequest): Promise<Json | void>;
}
export interface ReadoutClock {
  schedule(callback: () => void): () => void;
}
/** A section's own rows and its masks' rows. */
export function sectionRows(section: Section): Row[] {
  return [
    ...section.rows,
    ...(section.masks ?? []).flatMap((mask) => mask.rows),
  ];
}
export function rowBindings(row: Row): Binding[] {
  if (row.control.kind === "status") return [];
  const fields = "axes" in row.control ? row.control.axes : [row.control];
  return [
    ...fields.map((field) => field.binding),
    ...(row.control.kind === "colour" && row.control.hexBinding
      ? [row.control.hexBinding]
      : []),
    ...(row.expression ? [row.expression.binding] : []),
  ];
}
export function transaction(
  label: string,
  commands: EditCommand[],
): ModelOperation {
  return { type: "edit", label, commands };
}
