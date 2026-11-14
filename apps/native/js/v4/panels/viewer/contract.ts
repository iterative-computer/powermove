import { json, type Json } from "../../src/protocol";

/** Exact data projection produced by pm_panel_viewerchrome::v4::Session. */
export interface ViewerModel {
  ticket: string;
  project: string;
  composition: string | null;
  name: string;
  fit: boolean;
  shownZoom: number;
  quality: "auto" | "1" | "0.5" | "0.25";
  effectiveQuality: number;
  canvasLost: boolean;
  showControls: boolean;
  empty: boolean;
  selectionCount: number;
  /** Present when the composition has Studio 3D layers: the preview tier. */
  studio?: {
    tier: "live" | "refine";
    tool?: "move" | "rotate" | "scale" | "orbit" | "pan" | "zoom" | "light";
    look?: "camera" | "free";
    /** Handles follow the scene's axes or the object's own. */
    axes?: "scene" | "object";
    /** Panes in the viewer: one, two side by side, or four. */
    views?: "1" | "2" | "4";
    /** The viewer is narrower than the 3D group: show one pull-down. */
    compact?: boolean;
    /** Quality's refinement (3d-studio.md §3 status capsule). */
    refine?: {
      samplesDone: number;
      samplesTarget: number;
      stage: "sampling" | "denoising" | "complete";
    };
    /** Quality, paused, before the first progress report. */
    pending?: boolean;
    /** "guided" when this build has no Open Image Denoise (the engine's own filter). */
    denoiser?: "oidn" | "guided";
  } | null;
}
/** Observed host facts, never inferred from an enqueued intent. No pixel data. */
export interface ViewerHostFacts {
  presentation: "Docked" | "Focus" | "FullScreen";
  frame: { resource: string | null; loading: boolean; failed: boolean };
  source: null | {
    id: string;
    name: string;
    kind: "image" | "audio" | "video";
    time: number;
    duration: number | null;
    playing: boolean;
    epoch: string;
  };
  interaction: "idle" | "hover" | "focus" | "media-drop" | "effect-drop";
  /** The composition's transport, the same state the timeline header shows.
   * While playing, `timecode` stands still: a native layer draws the time.
   * Null while a timeline in the workspace shows play and the time. */
  transport?: {
    time: number;
    timecode: string;
    spokenTime: string;
    duration: string;
    playing: boolean;
    looping: boolean;
  } | null;
  /** The channel view, as the command that shows it (display only). */
  channels?: string;
}
export interface ViewerSnapshot {
  model: ViewerModel | null;
  host: ViewerHostFacts;
}
function record(v: Json): Record<string, Json> {
  if (!v || Array.isArray(v) || typeof v !== "object")
    throw new TypeError("Invalid viewer record");
  return v;
}
function text(v: Json | undefined): asserts v is string {
  if (typeof v !== "string" || !v.length)
    throw new TypeError("Invalid viewer identity/text");
}
function finite(v: Json | undefined): asserts v is number {
  if (typeof v !== "number" || !Number.isFinite(v))
    throw new TypeError("Invalid viewer number");
}
function decimal(v: Json | undefined) {
  text(v);
  if (!/^(0|[1-9][0-9]*)$/.test(v))
    throw new TypeError("Invalid viewer ticket");
}
function oneOf(v: Json | undefined, choices: Json[]) {
  if (!choices.includes(v ?? null)) throw new TypeError("Invalid viewer enum");
}
/** Validate before publishing. A malformed observation cannot replace good state. */
export function snapshot(value: unknown): ViewerSnapshot {
  const result = json(value);
  const root = record(result);
  if (root.model !== null) {
    const m = record(root.model);
    decimal(m.ticket);
    text(m.project);
    if (m.composition !== null) text(m.composition);
    if (typeof m.name !== "string")
      throw new TypeError("Invalid composition name");
    for (const k of ["fit", "canvasLost", "showControls", "empty"])
      if (typeof m[k] !== "boolean") throw new TypeError("Invalid viewer flag");
    finite(m.shownZoom);
    if (m.shownZoom < 0.02 || m.shownZoom > 8)
      throw new RangeError("Invalid zoom");
    oneOf(m.quality, ["auto", "1", "0.5", "0.25"]);
    finite(m.effectiveQuality);
    if (m.effectiveQuality < 0.25 || m.effectiveQuality > 1)
      throw new RangeError("Invalid effective quality");
    if (m.studio !== undefined && m.studio !== null)
      oneOf(record(m.studio).tier, ["live", "refine"]);
    finite(m.selectionCount);
    if (!Number.isSafeInteger(m.selectionCount) || m.selectionCount < 0)
      throw new RangeError("Invalid selection count");
  }
  const h = record(root.host);
  oneOf(h.presentation, ["Docked", "Focus", "FullScreen"]);
  oneOf(h.interaction, ["idle", "hover", "focus", "media-drop", "effect-drop"]);
  const frame = record(h.frame);
  if (frame.resource !== null) text(frame.resource);
  if (typeof frame.loading !== "boolean" || typeof frame.failed !== "boolean")
    throw new TypeError("Invalid frame observation");
  // Null (not just absent) while a timeline in the workspace owns play and
  // the timecode: the native host sends `"transport": null` then.
  if (h.transport !== undefined && h.transport !== null) {
    const t = record(h.transport);
    finite(t.time);
    for (const k of ["timecode", "spokenTime", "duration"])
      if (typeof t[k] !== "string") throw new TypeError("Invalid transport");
    if (typeof t.playing !== "boolean" || typeof t.looping !== "boolean")
      throw new TypeError("Invalid transport");
  }
  if (h.channels !== undefined)
    oneOf(
      h.channels,
      channelViews.map((c) => c.v),
    );
  if (h.source !== null) {
    if (root.model === null) throw new TypeError("Source requires a project");
    const s = record(h.source);
    text(s.id);
    text(s.name);
    decimal(s.epoch);
    oneOf(s.kind, ["image", "audio", "video"]);
    finite(s.time);
    if (s.time < 0 || typeof s.playing !== "boolean")
      throw new TypeError("Invalid source time");
    if (s.duration !== null) {
      finite(s.duration);
      if (s.duration <= 0 || s.time > s.duration)
        throw new RangeError("Invalid source duration");
    }
  }
  return result as unknown as ViewerSnapshot;
}
/** View › Channels: what the viewer shows of the picture, by command. */
export const channelViews = [
  { v: "viewer.showRGB", label: "RGB" },
  { v: "viewer.showRed", label: "Red" },
  { v: "viewer.showGreen", label: "Green" },
  { v: "viewer.showBlue", label: "Blue" },
  { v: "viewer.showAlpha", label: "Alpha" },
  { v: "viewer.showAlphaOverlay", label: "Alpha Overlay" },
];
export const zoomPresets = [0.05, 0.125, 0.25, 0.5, 1, 2, 4, 8] as const;
export const resolutions = [
  { v: "auto", label: "Auto" },
  { v: "1", label: "Full" },
  { v: "0.5", label: "Half" },
  { v: "0.25", label: "Quarter" },
];
export function percent(zoom: number) {
  const n = zoom * 100;
  return n === 12.5 ? "12.5" : String(Math.round(n));
}
export function zoomTitle(m: ViewerModel) {
  return m.fit
    ? `Fit (${percent(m.shownZoom)} %)`
    : `${percent(m.shownZoom)} %`;
}
/** Continuous adaptive scales are model facts, not invented menu choices. */
/** Named steps keep their names; an intermediate adaptive scale (e.g. 0.75)
 * shows as a percentage, `Auto (75 %)` (design review D2). */
export function effectiveResolution(m: ViewerModel): string | null {
  const named = (
    { "1": "Full", "0.5": "Half", "0.25": "Quarter" } as Record<string, string>
  )[String(m.effectiveQuality)];
  if (named) return named;
  const e = m.effectiveQuality;
  return typeof e === "number" && Number.isFinite(e) && e > 0 && e < 1
    ? `${percent(e)} %`
    : null;
}
export function resolutionTitle(m: ViewerModel) {
  const label = resolutions.find((o) => o.v === m.quality)!.label;
  const effective = effectiveResolution(m);
  return m.quality === "auto" && effective && effective !== "Full"
    ? `Auto (${effective})`
    : label;
}
