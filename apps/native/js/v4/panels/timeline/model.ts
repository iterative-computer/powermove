import type { NativeAction } from "./actions";
import type { EventDetail, Json } from "../../src/protocol";

/** Presentation boundary for pm-panel-timeline, not a second document model.
 * Native code must source this from TimelineClient/TimelineModel and await kernel
 * acknowledgements and HostEffects before resolving act(). There is deliberately
 * no JS fallback and no speculative host command namespace.
 */
export interface WindowRequest {
  first: number;
  visible: number;
}
export type Mode = "Layers" | "Tracks";
export type View = "DopeSheet" | "Combined" | "Graph";
export type Modifiers = Pick<
  EventDetail,
  "shiftKey" | "altKey" | "metaKey" | "ctrlKey"
>;
export type Announcement =
  | { kind: "markerAdded"; timecode: string }
  | { kind: "markerRemoved" }
  | { kind: "keyCollision" }
  | { kind: "outsideComposition" }
  | { kind: "locked" };
export function announcementText(value: Announcement): string {
  switch (value.kind) {
    case "markerAdded":
      return `Added marker at ${value.timecode}`;
    case "markerRemoved":
      return "Removed marker";
    case "keyCollision":
      return "Keyframe time is taken by another keyframe";
    case "outsideComposition":
      return "That time is outside the composition";
    case "locked":
      return "Layer is locked";
  }
}
export interface TextLease {
  /** Real, generation-scoped registered host binding; never a property path. */
  binding: string;
  value: string;
}
export interface NumericLease {
  binding: string;
  value: number;
  unit?: string;
  step: number;
  speed?: number;
  precision?: number;
  min?: number;
  max?: number;
}
interface RowBase {
  /** Stable native RowId encoding, not an ordinal or a layer display name. */
  id: string;
  label: string;
  /** Native, schema-aware, unit-spelled description from timeline.md §6. */
  accessibilityLabel: string;
  depth: number;
  selected: boolean;
  focused: boolean;
  hovered: boolean;
  locked: boolean;
}
export interface LayerRow extends RowBase {
  kind: "layer";
  layerKind?: string;
  labelColor?:
    | "blue"
    | "violet"
    | "orange"
    | "red"
    | "yellow"
    | "green"
    | "teal"
    | "gray";
  layer: string;
  index: number | null;
  canOpen: boolean;
  expanded: boolean;
  visible: boolean;
  solo: boolean;
  /** Shy: no row while Hide Shy Layers is on. */
  shy?: boolean;
  transformable: boolean;
  parent: { name: string; set: boolean };
  nameEditor?: TextLease;
  media: "ready" | "loading" | "offline";
}
export interface PropertyRow extends RowBase {
  kind: "property";
  layer: string;
  path: string;
  animated: boolean;
  keyAtPlayhead: boolean;
  previous: boolean;
  next: boolean;
  /** A compound Scale readout stays one semantic row, with real axis keys. */
  readout: string;
  numeric?: NumericLease;
  scaleLinked?: boolean;
}
export interface TrackRow extends RowBase {
  kind: "track";
  track: string;
  /** The clip's name, or how many clips the track holds. */
  summary?: string;
  audio: boolean;
  enabled: boolean;
  expanded: boolean;
}
export type Row = LayerRow | PropertyRow | TrackRow;
export interface TimelineFrame {
  /** Decimal native u64s: never round document/revision identities in JS. */
  document: string | null;
  revision: string;
  hasComposition: boolean;
  mode: Mode;
  view: View;
  playing: boolean;
  snapping: boolean;
  /** Hide Shy Layers is on; `shyLayers`: the composition has a shy layer. */
  hideShy?: boolean;
  shyLayers?: boolean;
  timecode: string;
  spokenTime: string;
  frameRate: string;
  timeEditor?: TextLease;
  tabs: { id: string; name: string; active: boolean; showClose: boolean }[];
  /** Already flattened/indexed by Rust. One viewport of overscan on each side. */
  window: { total: number; first: number; rows: readonly Row[] };
  /** Derived from largest index by Rust, independent of the mounted window. */
  indexDigits: number;
  selection: { kind: "layers" | "keys"; count: number } | null;
  renderSummary: string;
  announcement?: { sequence: string; value: Announcement };
  /** Semantic feedback only. Metadata is inspectable, never a paint API. */
  feedback: Json;
}
export type Intent =
  | NativeAction
  | { type: "play" }
  | { type: "open" }
  | { type: "import" }
  | { type: "mode"; mode: Mode }
  | { type: "view"; view: View }
  | { type: "snap" }
  | { type: "hideShy" }
  | { type: "graphOptions" }
  | { type: "timeEdit" }
  | { type: "tab"; id: string; close: boolean }
  | { type: "selectRow"; id: string; modifiers: Modifiers }
  | { type: "disclosure"; id: string; open: boolean }
  | { type: "rename"; id: string }
  /** A name or time field closed: its text, or empty when cancelled. */
  | { type: "editDone"; value: string }
  | {
      type: "switch";
      id: string;
      name: "visibility" | "lock" | "solo" | "shy" | "parent" | "mute";
    }
  | { type: "keyToggle"; layer: string; path: string }
  | {
      type: "adjacentKey";
      layer: string;
      path: string;
      direction: "previous" | "next";
    }
  | { type: "scaleLink"; layer: string }
  /** Pointer, wheel and drop gestures run in the native surface, not here. */
  | { type: "keyDown"; event: EventDetail }
  | { type: "range"; first: number; visible: number };

export interface NativeTimelineModel {
  /** Native adapter reads a single immutable kernel publication. Returned frames
   * must be detached presentation facts; no live document objects cross realms.
   * For WindowRequest {first,visible}, use model.window(first,visible,visible).
   * Native adapter owns ID anchoring across publications and physical hit tests.
   */
  inspect(window: WindowRequest): Promise<TimelineFrame>;
  /** Resolve only after kernel/transport/menu/import acknowledgement. Rejections
   * stay errors; no JS-side selection, preview, persistence or undo authority.
   */
  act(intent: Intent): Promise<void>;
  /** Await TimelineClient::dispose(), including active-gesture cancellation. */
  dispose(): Promise<void>;
}

export function windowRequest(first: number, visible: number): WindowRequest {
  if (
    !Number.isSafeInteger(first) ||
    first < 0 ||
    !Number.isSafeInteger(visible) ||
    visible < 1
  )
    throw new RangeError("Invalid timeline ordinal window");
  return { first, visible };
}

/** Bound work and fail closed on malformed native projections. No normalization
 * of authored values, IDs, subframes or unknown native feedback takes place.
 */
export function checkFrame(
  frame: TimelineFrame,
  request: WindowRequest,
): TimelineFrame {
  windowRequest(request.first, request.visible);
  const w = frame.window;
  if (
    [frame.hasComposition, frame.playing, frame.snapping].some(
      (value) => typeof value !== "boolean",
    ) ||
    [
      frame.timecode,
      frame.spokenTime,
      frame.frameRate,
      frame.renderSummary,
    ].some((value) => typeof value !== "string") ||
    !Array.isArray(w.rows) ||
    !Array.isArray(frame.tabs)
  )
    throw new TypeError("Invalid native presentation facts");
  if (
    typeof frame.revision !== "string" ||
    !/^\d+$/.test(frame.revision) ||
    (frame.document !== null &&
      (typeof frame.document !== "string" || !/^\d+$/.test(frame.document)))
  )
    throw new TypeError("Invalid native identity");
  if (
    !["Layers", "Tracks"].includes(frame.mode) ||
    !["DopeSheet", "Combined", "Graph"].includes(frame.view)
  )
    throw new TypeError("Invalid timeline mode/view");
  if (
    ![w.total, w.first].every((n) => Number.isSafeInteger(n) && n >= 0) ||
    w.first > w.total ||
    w.rows.length > w.total - w.first ||
    w.rows.length > request.visible * 3 ||
    !Number.isSafeInteger(frame.indexDigits) ||
    frame.indexDigits < 1 ||
    !["layers", "keys", undefined].includes(frame.selection?.kind) ||
    (frame.selection !== null &&
      (!Number.isSafeInteger(frame.selection.count) ||
        frame.selection.count < 0))
  )
    throw new RangeError("Invalid or unbounded timeline window");
  const ids = new Set<string>();
  for (const row of w.rows) {
    if (
      typeof row.id !== "string" ||
      !row.id ||
      typeof row.accessibilityLabel !== "string" ||
      !row.accessibilityLabel ||
      typeof row.label !== "string" ||
      [row.selected, row.focused, row.hovered, row.locked].some(
        (value) => typeof value !== "boolean",
      ) ||
      ids.has(row.id) ||
      !Number.isSafeInteger(row.depth) ||
      row.depth < 0 ||
      !["layer", "property", "track"].includes(row.kind)
    )
      throw new TypeError("Invalid native timeline row");
    ids.add(row.id);
  }
  const tabIds = new Set<string>();
  for (const tab of frame.tabs) {
    if (
      typeof tab.id !== "string" ||
      !tab.id ||
      tabIds.has(tab.id) ||
      typeof tab.name !== "string" ||
      typeof tab.active !== "boolean" ||
      typeof tab.showClose !== "boolean"
    )
      throw new TypeError("Invalid composition tab");
    tabIds.add(tab.id);
  }
  return frame;
}
