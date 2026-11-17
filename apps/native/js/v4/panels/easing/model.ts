/**
 * The Easing panel's model: what the native side says about the selected
 * keyframes (`easing.state`) and the actions it takes on them
 * (`panel.easing`, one Undo step each). The keyframes are the timeline's
 * selection; this panel keeps only its custom curve.
 */
export const PANEL_ID = "easing";
export const PANEL_TITLE = "Easing";
export const COMMAND = "panel.easing";

export type Curve = [number, number, number, number];
export interface Preset {
  id: string;
  curve: Curve;
}
export interface EasingState {
  open: boolean;
  /** Selected keyframes: what every action changes. */
  count: number;
  presets: Preset[];
  library?: { stateRevision: string; source: any };
  curve?: Curve | null;
  mixed?: boolean;
  interpolation?: Interpolation | null;
}
export type Side = "both" | "in" | "out";
export type Interpolation = "linear" | "bezier" | "hold" | "auto";
export type EasingAction =
  | { type: "preset"; preset: string }
  | { type: "save"; name: string; curve: Curve }
  | { type: "curve"; curve: Curve; name?: string }
  | { type: "easyEase"; side: Side }
  | { type: "interpolation"; interpolation: Interpolation; side: Side };

export const emptyState = (): EasingState => ({
  open: false,
  count: 0,
  presets: [],
});

/** A preset id as a person reads it: `easeInOut` reads "Ease In Out",
 * `expoOut` "Expo Out". */
export function presetName(id: string): string {
  const words = id.replace(/([a-z])([A-Z])/g, "$1 $2").split(" ");
  return words.map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
}

/** What the panel says under its title. */
export function status(state: EasingState, message: string): string {
  if (message) return message;
  if (!state.open) return "Open a project to ease its keyframes.";
  if (state.count === 0)
    return "Select keyframes in the timeline to ease them.";
  return state.count === 1
    ? "1 keyframe selected"
    : `${state.count} keyframes selected`;
}

/** Reads and drives the native side, one request at a time. */
export class EasingModel {
  private current = emptyState();
  private listeners = new Set<() => void>();
  private serial: Promise<unknown> = Promise.resolve();
  /** The last refusal or result, until the selection changes. */
  message = "";
  constructor(
    private readonly read: () => EasingState,
    private readonly run: (action: EasingAction) => Promise<unknown>,
    private readonly report: (error: unknown) => void,
  ) {}
  snapshot = () => this.current;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  settled() {
    return this.serial;
  }
  private notify() {
    this.listeners.forEach((f) => f());
  }
  refresh() {
    try {
      const next = this.read();
      if (next.count !== this.current.count) this.message = "";
      this.current = next;
    } catch (error) {
      this.report(error);
    }
    this.notify();
  }
  send(action: EasingAction) {
    this.serial = this.serial.then(async () => {
      try {
        await this.run(action);
        this.message = "";
      } catch (error) {
        this.message = String(
          error instanceof Error ? error.message : error,
        ).replace(/^Error: /, "");
      }
      this.refresh();
    });
  }
}

function librarySource(source: any) {
  return typeof source === "string" ? JSON.parse(source) : source;
}
/** The stored library remains authoritative; cubic tiles are a projection. */
export function customPresets(source: any): (Preset & { name: string })[] {
  let library;
  try {
    library = librarySource(source);
  } catch {
    return [];
  }
  if (!Array.isArray(library?.groups)) return [];
  return library.groups.flatMap((g: any) =>
    (Array.isArray(g?.presets) ? g.presets : []).flatMap((p: any) => {
      const points = p?.curve?.points;
      if (
        !Array.isArray(points) ||
        points.length !== 2 ||
        typeof p.id !== "string" ||
        typeof p.name !== "string"
      )
        return [];
      const [a, b] = points;
      if (
        !a ||
        !b ||
        !["linear", "bezier"].includes(a.outInterp) ||
        !["linear", "bezier"].includes(b.inInterp)
      )
        return [];
      // v1's resolved linear handle uses one third, regardless of stored ease.
      const x = a.outInterp === "linear" ? 1 / 3 : a.outEase?.influence / 100,
        incoming = b.inInterp === "linear" ? 1 / 3 : b.inEase?.influence / 100;
      const curve: Curve = [
        x,
        a.outInterp === "bezier" ? x * a.outEase?.speed : x,
        1 - incoming,
        b.inInterp === "bezier" ? 1 - incoming * b.inEase?.speed : 1 - incoming,
      ];
      return curve.every(Number.isFinite)
        ? [{ id: `saved:${p.id}`, name: p.name, curve }]
        : [];
    }),
  );
}
export function librarySave(state: EasingState, name: string, curve: Curve) {
  const point = (t: number, v: number) => ({
    t,
    v,
    inInterp: "bezier",
    outInterp: "bezier",
    inEase: { speed: 0, influence: 33 },
    outEase: { speed: 0, influence: 33 },
  });
  const a = point(0, 0),
    b = point(1, 1);
  a.outEase = {
    speed: curve[1] / Math.max(0.001, curve[0]),
    influence: Math.max(0.1, curve[0] * 100),
  };
  b.inEase = {
    speed: (1 - curve[3]) / Math.max(0.001, 1 - curve[2]),
    influence: Math.max(0.1, (1 - curve[2]) * 100),
  };
  const existing = librarySource(state.library?.source);
  if (state.library?.source != null && !Array.isArray(existing?.groups))
    throw new Error(
      "The saved easing library could not be read; it has been retained.",
    );
  const source = structuredClone(
    existing ?? {
      version: 1,
      active: "custom",
      groups: [{ id: "custom", name: "Custom", presets: [] }],
    },
  );
  const group =
    source.groups.find((g: any) => g.id === source.active) ?? source.groups[0];
  const preset = {
    id: `curve-${Date.now()}`,
    name,
    color: "",
    curve: { points: [a, b] },
  };
  if (state.library?.source)
    return {
      type: "easingLibrary",
      action: "save",
      stateRevision: state.library.stateRevision,
      group: group.id,
      ...preset,
    };
  group.presets.push(preset);
  return {
    type: "easingLibrary",
    action: "import",
    stateRevision: state.library?.stateRevision,
    source,
    ids: [],
  };
}
