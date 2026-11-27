/**
 * The Easing panel's model: what the native side says about the selected
 * keyframes (`easing.state`) and the actions it takes on them
 * (`panel.easing`, one Undo step each). The keyframes are the timeline's
 * selection. The panel can also edit and save a draft curve.
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
  /** Number of consecutive keyframe segments covered by the selection. */
  segments?: number;
  selection?: [number, string[], [string, string][]];
  presets: Preset[];
  library?: { stateRevision: string; source: any };
  curve?: Curve | null;
  mixed?: boolean;
  interpolation?: Interpolation | null;
  mixedInterpolation?: boolean;
}
export type Side = "both" | "in" | "out";
export type ApplyScope = "between" | "all";
export type Interpolation =
  | "linear"
  | "bezier"
  | "hold"
  | "auto"
  | "continuous";
export type EasingAction =
  | { type: "preset"; preset: string; scope?: ApplyScope }
  | { type: "save"; group: string; name: string; curve: Curve }
  | { type: "addGroup"; id: string; name: string }
  | { type: "curve"; curve: Curve; name?: string; scope?: ApplyScope }
  | { type: "easyEase"; side: Side; scope?: ApplyScope }
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
  if (state.count === 0) return "Select keyframes to apply easing";
  const count =
    state.count === 1
      ? "1 keyframe selected"
      : `${state.count} keyframes selected`;
  const seg =
    state.segments && state.segments > 0
      ? state.segments === 1
        ? " · 1 segment"
        : ` · ${state.segments} segments`
      : "";
  return state.mixed ? `${count}${seg} · Mixed curves` : `${count}${seg}`;
}

export function selectionKey(state: EasingState): string {
  return JSON.stringify(state.selection ?? state.count);
}

/** Reads and drives the native side, one request at a time. */
export class EasingModel {
  private current = emptyState();
  private listeners = new Set<() => void>();
  private serial: Promise<unknown> = Promise.resolve();
  private pendingEdits = 0;
  get applying() {
    return this.pendingEdits > 0;
  }
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
      if (selectionKey(next) !== selectionKey(this.current)) this.message = "";
      this.current = next;
    } catch (error) {
      this.report(error);
    }
    this.notify();
  }
  send(action: EasingAction) {
    const library = action.type === "save" || action.type === "addGroup";
    const selection = selectionKey(this.current);
    if (!library) this.pendingEdits++;
    this.serial = this.serial.then(async () => {
      try {
        if (!library && selection !== selectionKey(this.current)) return;
        await this.run(action);
        this.message = action.type === "save" ? "Preset saved" : "";
      } catch (error) {
        this.message = String(
          error instanceof Error ? error.message : error,
        ).replace(/^Error: /, "");
      } finally {
        if (!library) this.pendingEdits--;
        this.refresh();
      }
    });
  }
}

const DEFAULT_GROUP = "default";

function emptyLibrary() {
  return {
    version: 1,
    active: DEFAULT_GROUP,
    groups: [{ id: DEFAULT_GROUP, name: "1", presets: [] }],
  };
}

function librarySource(source: any) {
  return typeof source === "string" ? JSON.parse(source) : source;
}
/** The stored library remains authoritative; cubic tiles are a projection. */
export function customPresets(source: any, group?: string): NamedPreset[] {
  let library;
  try {
    library = librarySource(source);
  } catch {
    return [];
  }
  if (!Array.isArray(library?.groups)) return [];
  return library.groups
    .filter((g: any) => group === undefined || g?.id === group)
    .flatMap((g: any) =>
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
          incoming =
            b.inInterp === "linear" ? 1 / 3 : b.inEase?.influence / 100;
        const curve: Curve = [
          x,
          a.outInterp === "bezier" ? x * a.outEase?.speed : x,
          1 - incoming,
          b.inInterp === "bezier"
            ? 1 - incoming * b.inEase?.speed
            : 1 - incoming,
        ];
        return curve.every(Number.isFinite)
          ? [{ id: `saved:${p.id}`, name: p.name, curve }]
          : [];
      }),
    );
}
type NamedPreset = Preset & { name: string };
export type PresetTile = NamedPreset & { native?: boolean };
export interface PresetGroup {
  id: string;
  name: string;
  presets: PresetTile[];
}

const electronCurves: NamedPreset[] = [
  ...[33, 50, 66, 80, 90].map(
    (influence): NamedPreset => ({
      id: `ease-${influence}`,
      name: `Ease ${influence}`,
      curve: [influence / 100, 0, (100 - influence) / 100, 1],
    }),
  ),
  { id: "steep-in", name: "Ease In", curve: [0.7, 0, 1, 1] },
  { id: "steep-out", name: "Ease Out", curve: [0, 0, 0.3, 1] },
  { id: "soft", name: "Soft", curve: [0.2, 0, 0.2, 1] },
  { id: "smooth", name: "Smooth", curve: [0.6, 0, 0.1, 1] },
  { id: "fast-out", name: "Fast Out", curve: [0, 0, 0.1, 1] },
  { id: "fast-in", name: "Fast In", curve: [0.9, 0, 1, 1] },
];

export function presetGroups(state: EasingState): {
  groups: PresetGroup[];
  active?: string;
} {
  let library;
  try {
    library = librarySource(state.library?.source);
  } catch {
    // Malformed stored data remains untouched.
  }
  const groups: PresetGroup[] = Array.isArray(library?.groups)
    ? library.groups.flatMap((g: any) =>
        typeof g?.id === "string" && typeof g.name === "string"
          ? [{ id: g.id, name: g.name, presets: customPresets(library, g.id) }]
          : [],
      )
    : [];
  const first = groups.find((g) => g.id === DEFAULT_GROUP) ??
    groups.find((g) => g.id === "custom" && g.name === "Custom") ?? {
      id: DEFAULT_GROUP,
      name: "1",
      presets: [],
    };
  return {
    active: library?.active,
    groups: [
      {
        id: first.id,
        name: "1",
        presets: [
          ...electronCurves,
          ...state.presets.map((p) => ({
            ...p,
            name: presetName(p.id),
            native: true,
          })),
          ...first.presets,
        ],
      },
      ...groups.filter((g) => g.id !== first.id),
    ],
  };
}

export function libraryAddGroup(state: EasingState, id: string, name: string) {
  const base = {
    type: "easingLibrary",
    stateRevision: state.library?.stateRevision,
  };
  if (state.library?.source != null)
    return { ...base, action: "addGroup", id, name };
  const source = emptyLibrary();
  source.groups.push({ id, name, presets: [] });
  source.active = id;
  return { ...base, action: "import", source, ids: [] };
}

export function librarySave(
  state: EasingState,
  name: string,
  curve: Curve,
  groupId?: string,
) {
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
  const source = existing ?? emptyLibrary();
  const group =
    groupId ??
    source.groups.find((g: any) => g.id === source.active)?.id ??
    source.groups[0].id;
  const preset = {
    id: `curve-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    name,
    color: "",
    curve: { points: [a, b] },
  };
  if (existing)
    return {
      type: "easingLibrary",
      action: "save",
      stateRevision: state.library.stateRevision,
      group,
      ...preset,
    };
  source.groups[0].presets.push(preset);
  return {
    type: "easingLibrary",
    action: "import",
    stateRevision: state.library?.stateRevision,
    source,
    ids: [],
  };
}
