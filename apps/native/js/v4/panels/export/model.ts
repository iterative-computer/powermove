import { HostClient, json, type Json } from "../../src";
import {
  actions,
  extensions,
  formats,
  kindOf,
  sizeError,
  type Format,
  type Kind,
} from "./spec";

/** A projection of pm-promedia + engine facts, never a writable project mirror.
 * The native KernelData adapter must implement this channel; see README API gaps. */
export interface Options {
  [key: string]: Json;
  format: Format;
  scale: number | "custom";
  width: number;
  height: number;
  fps: number;
  range: "work" | "all";
  quality: "draft" | "high" | "max";
  mblur: boolean;
  audio: boolean;
  alpha: boolean;
  captionsBurn: boolean;
  captionsSidecar: "none" | "srt" | "vtt";
}
export type Run =
  | {
      phase:
        | "idle"
        | "choosingDestination"
        | "preparing"
        | "finishing"
        | "cancelling";
    }
  | {
      phase: "rendering";
      frame: number;
      total: number;
      elapsed: number;
      eta?: number;
      extra?: string;
      preview?: string;
    }
  | { phase: "done"; path: string }
  | { phase: "failed"; message: string }
  | { phase: "cancelled" };
export interface ExportSnapshot {
  version: 1;
  /** Monotonic export-observation sequence, independent of document revision. */
  sequence: number;
  project: null | {
    id: string;
    name: string;
    width: number;
    height: number;
    hasAudio: boolean;
    captionLayers: number;
    hasWorkArea: boolean;
    /** Host planner uses engine tick/rate/range semantics, including still timecode. */
    summary: string;
    note?: string;
  };
  options: Options;
  /** Native generation-owned draft bindings, registered before this projection. */
  bindings: Record<string, string>;
  presets: { id: string; name: string; options: Options }[];
  capabilities: Record<Format, { available: boolean; reason?: string }>;
  run: Run;
}
const object = (x: unknown): x is Record<string, unknown> =>
  !!x && typeof x === "object" && !Array.isArray(x);
function validOptions(x: unknown): x is Options {
  if (!object(x) || !formats.some((f) => f.v === x.format)) return false;
  return (
    (x.scale === "custom" || [0.5, 1, 2].includes(x.scale as number)) &&
    [x.width, x.height, x.fps].every(
      (v) => typeof v === "number" && Number.isFinite(v) && v > 0,
    ) &&
    ["work", "all"].includes(x.range as string) &&
    ["draft", "high", "max"].includes(x.quality as string) &&
    ["none", "srt", "vtt"].includes(x.captionsSidecar as string) &&
    [x.mblur, x.audio, x.alpha, x.captionsBurn].every(
      (v) => typeof v === "boolean",
    )
  );
}
export function snapshot(value: unknown): ExportSnapshot {
  json(value);
  if (
    !object(value) ||
    value.version !== 1 ||
    !Number.isSafeInteger(value.sequence) ||
    (value.sequence as number) < 0 ||
    !validOptions(value.options) ||
    !object(value.bindings) ||
    ![
      "format",
      "scale",
      "width",
      "height",
      "fps",
      "range",
      "quality",
      "mblur",
      "audio",
      "alpha",
      "captionsBurn",
      "captionsSidecar",
      "preset",
      "presetName",
    ].every(
      (k) =>
        typeof (value.bindings as Record<string, unknown>)[k] === "string" &&
        (value.bindings as Record<string, string>)[k].length > 0,
    ) ||
    !Array.isArray(value.presets) ||
    !value.presets.every(
      (p) =>
        object(p) &&
        typeof p.id === "string" &&
        typeof p.name === "string" &&
        validOptions(p.options),
    ) ||
    !object(value.capabilities) ||
    !formats.every((f) => {
      const c = (value.capabilities as Record<string, unknown>)[f.v];
      return (
        object(c) &&
        typeof c.available === "boolean" &&
        (c.available || typeof c.reason === "string")
      );
    }) ||
    !object(value.run)
  )
    throw new TypeError("Invalid export projection");
  const p = value.project;
  if (
    p !== null &&
    (!object(p) ||
      typeof p.id !== "string" ||
      typeof p.name !== "string" ||
      typeof p.summary !== "string" ||
      typeof p.hasAudio !== "boolean" ||
      typeof p.hasWorkArea !== "boolean" ||
      !Number.isSafeInteger(p.captionLayers) ||
      (p.captionLayers as number) < 0 ||
      ![p.width, p.height].every(
        (v) => typeof v === "number" && v > 0 && Number.isFinite(v),
      ))
  )
    throw new TypeError("Invalid export project facts");
  const r = value.run;
  if (
    ![
      "idle",
      "choosingDestination",
      "preparing",
      "rendering",
      "finishing",
      "cancelling",
      "done",
      "failed",
      "cancelled",
    ].includes(r.phase as string) ||
    (r.phase === "rendering" &&
      (!Number.isSafeInteger(r.frame) ||
        !Number.isSafeInteger(r.total) ||
        (r.total as number) <= 0 ||
        (r.frame as number) < 0 ||
        (r.frame as number) > (r.total as number) ||
        typeof r.elapsed !== "number" ||
        r.elapsed < 0 ||
        (r.eta !== undefined && (typeof r.eta !== "number" || r.eta < 0)))) ||
    (r.phase === "done" && typeof r.path !== "string") ||
    (r.phase === "failed" && typeof r.message !== "string")
  )
    throw new TypeError("Invalid export run");
  return structuredClone(value) as unknown as ExportSnapshot;
}
export const isProgress = (run: Run) =>
  ["preparing", "rendering", "finishing", "cancelling", "failed"].includes(
    run.phase,
  );
export function progress(run: Run): {
  leading: string;
  eta?: string;
  value?: number;
  ax?: string;
} {
  switch (run.phase) {
    case "rendering": {
      const pct = Math.round((run.frame / run.total) * 100);
      const eta =
        run.frame >= 8 && run.elapsed >= 1 && run.eta !== undefined
          ? run.eta >= 90
            ? `About ${Math.round(run.eta / 60)} min left`
            : `About ${Math.max(1, Math.round(run.eta))} s left`
          : undefined;
      return {
        leading: `Frame ${run.frame} of ${run.total} · ${pct} %${run.extra ? ` · ${run.extra}` : ""}`,
        eta,
        value: run.frame / run.total,
        ax: `${pct} percent, frame ${run.frame} of ${run.total}`,
      };
    }
    case "finishing":
      return { leading: "Finishing…", value: 1 };
    case "cancelling":
      return { leading: "Cancelling…" };
    default:
      return { leading: "Preparing…" };
  }
}

/** Worker-side sheet draft. All persisted state, destinations, jobs and completion
 * effects go through HostClient to the single native writer/worker. No UI wait. */
export class ExportModel {
  private current: ExportSnapshot;
  private listeners = new Set<() => void>();
  private stop?: () => Promise<void>;
  private disposed = false;
  private busy = false;
  private draftVersion = 0;
  private planVersion = 0;
  private readyPlanVersion = 0;
  private remembered: Record<Kind, Format> = {
    video: "mp4",
    images: "still",
    project: "json",
  };
  options: Options;
  private sizeDraft?: { width: number; height: number };
  get size() {
    return (
      this.sizeDraft ?? {
        width: this.options.width,
        height: this.options.height,
      }
    );
  }
  presetName = "";
  chosenPreset = "";
  open = false;
  error?: string;
  /** Transport/projection failures are inspectable; no unapproved error copy. */
  fault?: unknown;
  constructor(
    readonly host: HostClient,
    initial: ExportSnapshot,
  ) {
    this.current = snapshot(initial);
    this.options = structuredClone(initial.options);
    this.remembered[kindOf(this.options.format)] = this.options.format;
    this.validateSize();
  }
  get state() {
    return structuredClone(this.current);
  }
  get kind() {
    return kindOf(this.options.format);
  }
  get pending() {
    return this.busy;
  }
  get locked() {
    return this.busy || this.current.run.phase !== "idle";
  }
  get canExport() {
    return (
      !!this.current.project &&
      !this.locked &&
      !this.error &&
      !this.fault &&
      this.readyPlanVersion === this.planVersion &&
      this.current.capabilities[this.options.format].available
    );
  }
  get actionLabel() {
    return actions[this.options.format];
  }
  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  private changed() {
    for (const listener of this.listeners) listener();
  }
  private accept(value: unknown, replaceDraft = false) {
    if (this.disposed) return;
    let next: ExportSnapshot;
    try {
      next = snapshot(value);
    } catch (error) {
      this.fault = error;
      this.changed();
      throw error;
    }
    if (next.sequence < this.current.sequence) return;
    if (this.current.project?.id !== next.project?.id) {
      this.open = false;
      this.draftVersion++;
      replaceDraft = true;
    }
    this.current = next;
    if (replaceDraft) {
      this.options = structuredClone(next.options);
      this.sizeDraft = undefined;
      this.remembered[kindOf(this.options.format)] = this.options.format;
      this.validateSize();
    }
    if (next.run.phase === "done" || next.run.phase === "cancelled")
      this.open = false;
    this.changed();
  }
  async connect(subscriptionId: string) {
    this.stop = await this.host.subscribe(
      subscriptionId,
      "project",
      (v) => this.accept(v, this.readyPlanVersion === this.planVersion),
      ["export"],
    );
    if (this.disposed) await this.stop();
  }
  async command(id: string, args: Json = {}) {
    if (this.disposed) throw new Error("Export panel retired");
    try {
      return await this.host.command(id, args);
    } catch (error) {
      this.fault = error;
      this.changed();
      throw error;
    }
  }
  async show() {
    if (!this.current.project || this.disposed) return;
    if (
      isProgress(this.current.run) ||
      this.current.run.phase === "choosingDestination"
    ) {
      this.open = true;
      this.changed();
      await this.command("export.reveal");
      return;
    }
    if (["done", "cancelled"].includes(this.current.run.phase))
      this.accept((await this.command("export.reset")).value, true);
    this.open = true;
    this.changed();
  }
  set<K extends keyof Options>(key: K, value: Options[K]) {
    if (this.locked || this.disposed) return;
    const next = { ...this.options, [key]: value };
    if (!validOptions(next)) throw new TypeError("Invalid export option");
    if (key === "format" && !this.current.capabilities[next.format].available)
      return;
    if (key === "audio" && !this.current.project?.hasAudio) return;
    this.options = next;
    if (key === "scale") this.sizeDraft = undefined;
    this.remembered[this.kind] = this.options.format;
    this.draftVersion++;
    this.readyPlanVersion = -1;
    this.validateSize();
    this.changed();
  }
  chooseKind(kind: Kind) {
    if (this.locked || this.disposed) return;
    // Changing kind must still expose an unavailable remembered format, with
    // its reason and a disabled default action; never choose a fallback codec.
    this.options = { ...this.options, format: this.remembered[kind] };
    this.draftVersion++;
    this.readyPlanVersion = -1;
    this.validateSize();
    this.changed();
  }
  customSize(axis: "width" | "height", value: number) {
    if (this.locked || this.disposed || !this.current.project) return;
    if (!Number.isFinite(value)) {
      this.error = sizeError;
      this.changed();
      return;
    }
    const p = this.current.project;
    const other = axis === "width" ? "height" : "width";
    const pair = {
      ...this.options,
      [axis]: value,
      [other]: Math.round((value * p[other]) / p[axis]),
    };
    this.sizeDraft = { width: pair.width, height: pair.height };
    this.options = { ...this.options, scale: "custom" };
    // Invalid native text drafts are inspectable UI state, never remembered
    // render options. Project delivery can always ignore hidden raster fields.
    if (
      [pair.width, pair.height].every(
        (n) => Number.isInteger(n) && n >= 16 && n <= 8192,
      )
    )
      this.options = {
        ...this.options,
        width: pair.width,
        height: pair.height,
      };
    this.draftVersion++;
    this.readyPlanVersion = -1;
    this.validateSize();
    this.changed();
  }
  private validateSize() {
    this.error =
      this.options.format !== "json" &&
      this.options.scale === "custom" &&
      ![this.size.width, this.size.height].every(
        (n) => Number.isInteger(n) && n >= 16 && n <= 8192,
      )
        ? sizeError
        : undefined;
  }
  /** Planner is native: exact ticks, endpoint rules and codec constraints stay in Rust.
   * Older async plan acks cannot overwrite a newer draft/summary. */
  async refreshPlan() {
    const version = ++this.planVersion;
    const draft = this.draftVersion;
    if (this.error) return;
    const ack = await this.command("export.plan", {
      options: json(this.options),
    });
    if (
      this.disposed ||
      version !== this.planVersion ||
      draft !== this.draftVersion
    )
      return;
    this.accept(ack.value, true);
    this.readyPlanVersion = version;
    this.changed();
  }
  async choosePreset(id: string) {
    if (this.locked) return;
    const preset = this.current.presets.find((p) => p.id === id);
    if (!preset || !this.current.capabilities[preset.options.format].available)
      return;
    this.options = structuredClone(preset.options);
    this.sizeDraft = undefined;
    this.remembered[this.kind] = this.options.format;
    this.chosenPreset = id;
    this.draftVersion++;
    this.readyPlanVersion = -1;
    this.validateSize();
    this.changed();
    await this.refreshPlan();
  }
  namePreset(name: string) {
    if (!this.locked) {
      this.presetName = name;
      this.changed();
    }
  }
  async savePreset() {
    if (this.locked || !this.presetName.trim() || this.error) return;
    this.busy = true;
    this.changed();
    try {
      // Native actor appends a fresh ID, archives same-name predecessors (N4).
      const ack = await this.command("export.preset.save", {
        name: this.presetName.trim(),
        options: json(this.options),
      });
      this.accept(ack.value);
      this.chosenPreset =
        [...this.current.presets]
          .reverse()
          .find((p) => p.name === this.presetName.trim())?.id ??
        this.chosenPreset;
    } finally {
      this.busy = false;
      this.changed();
    }
  }
  async start() {
    if (!this.canExport) return;
    this.busy = true;
    this.current.run = { phase: "choosingDestination" };
    this.changed();
    try {
      const ack = await this.command("export.begin", {
        options: json(this.options),
        destination: {
          kind: this.options.format === "png" ? "folder" : "file",
          suggestedName: `${this.current.project!.name}.${extensions[this.options.format]}`,
        },
      });
      this.accept(ack.value);
    } catch (error) {
      if (this.current.run.phase === "choosingDestination")
        this.current.run = { phase: "idle" };
      throw error;
    } finally {
      this.busy = false;
      this.changed();
    }
  }
  async cancel() {
    if (this.current.run.phase === "idle") {
      this.open = false;
      this.changed();
      return;
    }
    if (
      ["preparing", "rendering", "finishing"].includes(this.current.run.phase)
    ) {
      this.current.run = { phase: "cancelling" };
      this.changed();
      this.accept((await this.command("export.cancel")).value);
    }
  }
  async closeFailure() {
    if (this.current.run.phase !== "failed" || this.busy) return;
    this.accept((await this.command("export.closeFailure")).value);
    this.open = false;
    this.changed();
  }
  async retry() {
    if (this.current.run.phase !== "failed" || this.busy || this.fault) return;
    this.busy = true;
    this.changed();
    try {
      this.accept((await this.command("export.retry")).value);
    } finally {
      this.busy = false;
      this.changed();
    }
  }
  async key(key: string, meta = false, shift = false, presetField = false) {
    if (meta && key.toLowerCase() === "e") {
      await this.show();
      return true;
    }
    if (!this.open) return false;
    if (presetField && key === "Enter") {
      await this.savePreset();
      return true;
    }
    if (key === "Escape" && this.current.run.phase === "idle") {
      await this.cancel();
      return true;
    }
    if (meta && key === "." && isProgress(this.current.run)) {
      await this.cancel();
      return true;
    }
    if (key === "Enter" && !isProgress(this.current.run)) {
      await this.start();
      return true;
    }
    if (key === "Enter" && this.current.run.phase === "failed") {
      await this.retry();
      return true;
    }
    void shift; // Both shipped export shortcuts; shell owns registry arbitration.
    return false;
  }
  async dispose() {
    this.disposed = true;
    this.listeners.clear();
    await this.stop?.();
  }
}
