/**
 * The layer's effects as the Effect Controls model projects them
 * (`panel:layer-effects`, driven by `panel.effect-controls`), read in this
 * panel's own realm. Only what the Color panel uses is typed here.
 */
import { HostClient } from "../../src/host-api";
import { json, type Json } from "../../src/protocol";
import { groups } from "./contract";

export const MODEL_COMMAND = "panel.effect-controls";
export const MODEL_CHANNEL = "panel:layer-effects";

export interface Parameter {
  id: string;
  effect: string;
  /** The definition's parameter key. */
  key?: string;
  label: string;
  binding: string;
  value: Json;
  control: "number" | "text" | "toggle" | "color" | "choice" | "opaque";
  animated: boolean;
  keyAtPlayhead: boolean;
  mixed: boolean;
  readonly: boolean;
  step?: number;
  min?: number;
  max?: number;
  precision?: number;
  unit?: string;
  options?: { v: Json; label: string; disabled?: boolean }[];
}
export interface EffectHeader {
  kind: "effect";
  id: string;
  /** The effect's definition id (`type`). */
  definition?: string;
  /** Its definition says `grade: true`. */
  grade?: boolean;
  label: string;
  enabled: boolean;
  expanded: boolean;
}
export interface ParameterRow {
  kind: "parameter";
  id: string;
  parameter: Parameter;
  enabled: boolean;
}
export type Row = EffectHeader | ParameterRow | { kind: "action"; id: string };
export interface Projection {
  target: "NoProject" | "NoSelection" | "Layer" | "Unsupported";
  layer: string | null;
  name: string;
  typeLabel: string;
  locked: boolean;
  revision: number;
  documentId: number;
  count: number;
  range: [number, number];
  rows: Row[];
}
export type Intent =
  | { type: "range"; range: [number, number] }
  | { type: "disclose"; id: string; open: boolean }
  | { type: "toggle" | "reset" | "remove"; id: string }
  | { type: "add"; definition: string }
  | {
      type: "parameter";
      id: string;
      action: "diamond" | "stopwatch" | "reset";
    };
export interface Platform {
  contextMenu(items: { id: string; label: string }[]): Promise<string | null>;
  report(error: unknown): void;
}

export const emptyProjection = (): Projection => ({
  target: "NoProject",
  layer: null,
  name: "",
  typeLabel: "",
  locked: false,
  revision: 0,
  documentId: 0,
  count: 0,
  range: [0, 0],
  rows: [],
});

/** The model, its native field bindings and its intents, one at a time. */
export class ColorModel {
  private current = emptyProjection();
  private listeners = new Set<() => void>();
  private bindings = new Set<string>();
  private serial: Promise<unknown> = Promise.resolve();
  private stop?: () => Promise<void>;
  private disposed = false;
  constructor(
    readonly host: HostClient,
    readonly platform: Platform,
  ) {}
  snapshot = () => structuredClone(this.current);
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  settled() {
    return this.serial;
  }
  private enqueue<T>(job: () => Promise<T>): Promise<T> {
    const pending = this.serial.then(job);
    this.serial = pending.catch((error) => this.platform.report(error));
    return pending;
  }
  private async adopt(value: unknown) {
    if (this.disposed) return;
    const next = json(value) as unknown as Projection;
    if (
      !next ||
      !Array.isArray(next.rows) ||
      next.revision < this.current.revision
    )
      return;
    const wanted = new Map<string, string[]>();
    if (!next.locked) {
      for (const r of next.rows)
        if (r.kind === "parameter" && !r.parameter.readonly)
          wanted.set(r.parameter.binding, [r.parameter.id]);
      // A wheel's red, green and blue, a curve's points: one write each.
      for (const g of groups(next)) wanted.set(g.binding, g.channels);
    }
    for (const [handle, channels] of wanted)
      if (!this.bindings.has(handle)) {
        await this.host.registerBinding(handle, channels);
        this.bindings.add(handle);
      }
    // A retired binding cancels any gesture it captured.
    for (const handle of this.bindings)
      if (!wanted.has(handle)) {
        await this.host.disposeBinding(handle);
        this.bindings.delete(handle);
      }
    this.current = next;
    this.listeners.forEach((f) => f());
  }
  async start(subscription: string) {
    this.stop = await this.host.subscribe(
      subscription,
      "evaluated",
      (value) => {
        void this.enqueue(() => this.adopt(value));
      },
      [MODEL_CHANNEL],
    );
    await this.serial;
  }
  /** Drives the model, fenced to the layer and document it was asked about. */
  send(intent: Intent) {
    const asked = this.current;
    void this.enqueue(async () => {
      if (
        this.disposed ||
        asked.layer !== this.current.layer ||
        asked.documentId !== this.current.documentId
      )
        return;
      const ack = await this.host.command(
        MODEL_COMMAND,
        json({
          layer: asked.layer,
          revision: this.current.revision,
          documentId: asked.documentId,
          intent,
        } as unknown as Json),
      );
      if (ack.value !== undefined) await this.adopt(ack.value);
    });
  }
  /** A control's menu: Reset clears its keys and expression. */
  async controlMenu(row: ParameterRow) {
    if (this.current.locked || row.parameter.readonly) return;
    const choice = await this.platform.contextMenu([
      { id: "reset", label: "Reset" },
    ]);
    if (choice === "reset")
      this.send({ type: "parameter", id: row.parameter.id, action: "reset" });
  }
  async dispose() {
    this.disposed = true;
    await this.serial;
    await this.stop?.();
    for (const handle of this.bindings) await this.host.disposeBinding(handle);
    this.bindings.clear();
    this.listeners.clear();
  }
}
