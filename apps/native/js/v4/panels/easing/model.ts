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
}
export type Side = "both" | "in" | "out";
export type Interpolation = "linear" | "bezier" | "hold" | "auto";
export type EasingAction =
  | { type: "preset"; preset: string }
  | { type: "curve"; curve: Curve }
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
