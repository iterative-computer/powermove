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
  | ({ kind: "segmented"; options: SegmentOption[] } & Field<string>);
export interface Row {
  id: string;
  label: string;
  control: Control;
  locked?: boolean;
  disabled?: boolean;
  animation?: "static" | "animated" | "key-at-playhead";
  linked?: boolean;
  expression?: { binding: Binding; source: string; error: string | null };
  error?: { kind: "number" } | { kind: "range"; min: number; max: number };
}
export type SectionAction = "mask" | "animator" | "release-precomp";
export interface Section {
  id: string;
  title: string;
  collapsed: boolean;
  rows: Row[];
  action?: SectionAction;
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
  | { type: "collapse"; section: string; collapsed: boolean }
  | { type: "section-action"; section: string; action: SectionAction }
  | { type: "drop-effect"; effect: Json; target: string }
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
export function rowBindings(row: Row): Binding[] {
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
