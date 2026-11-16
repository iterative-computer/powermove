import { HostClient } from "../../src/host-api";
import { json } from "../../src/protocol";
import {
  PANEL_ID,
  MODEL_CHANNEL,
  MODEL_COMMAND,
  emptyProjection,
  projection,
  type Intent,
  type Platform,
  type Projection,
  type EffectHeader,
  type MenuItem,
} from "./contract";

/** All model mutations run on the worker through the acknowledged host service. */
export class EffectsController {
  outside(insideEffectSurface: boolean) {
    if (!insideEffectSurface) this.send({ type: "clear" });
  }
  async settled() {
    await this.serial;
  }
  private current = emptyProjection();
  private listeners = new Set<() => void>();
  private stops: (() => Promise<void>)[] = [];
  private bindings = new Set<string>();
  private serial: Promise<unknown> = Promise.resolve();
  private disposed = false;
  private started = false;
  constructor(
    readonly host: HostClient,
    readonly platform: Platform,
    private readonly subscriptionId: string,
  ) {}
  snapshot = () => structuredClone(this.current);
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private async adopt(value: unknown) {
    if (this.disposed) return;
    const next = projection(json(value));
    if (next.revision < this.current.revision) return;
    // A parameter's binding, and each one a summary row edits (a wheel's
    // master field).
    const editable = new Map<string, string>();
    if (!next.locked)
      for (const r of next.rows) {
        const list =
          r.kind === "parameter"
            ? [r.parameter]
            : r.kind === "summary"
              ? r.parameters
              : [];
        for (const parameter of list)
          if (!parameter.readonly)
            editable.set(parameter.binding, parameter.id);
      }
    const wanted = new Set(editable.keys());
    for (const [handle, id] of editable)
      if (!this.bindings.has(handle)) {
        await this.host.registerBinding(handle, [id]);
        this.bindings.add(handle);
      }
    // Native binding disposal must cancel any captured gesture before retirement.
    for (const handle of this.bindings)
      if (!wanted.has(handle)) {
        await this.host.disposeBinding(handle);
        this.bindings.delete(handle);
      }
    if (this.disposed) return;
    this.current = next;
    this.listeners.forEach((f) => f());
  }
  private enqueue<T>(job: () => Promise<T>): Promise<T> {
    const pending = this.serial.then(job);
    this.serial = pending.catch((error) => this.platform.report(error));
    return pending;
  }
  async start() {
    if (this.started || this.disposed)
      throw new Error("Effect controls lifecycle already started");
    this.started = true;
    const stop = await this.host.subscribe(
      this.subscriptionId,
      "evaluated",
      (value) => {
        void this.enqueue(() => this.adopt(value));
      },
      [MODEL_CHANNEL],
    );
    this.stops.push(stop);
    await this.serial;
  }
  async drive(intent: Intent) {
    intent = json(intent) as unknown as Intent;
    const capturedLayer = this.current.layer;
    const capturedDocument = this.current.documentId;
    return this.enqueue(async () => {
      if (
        capturedLayer !== this.current.layer ||
        capturedDocument !== this.current.documentId
      )
        throw new Error("Effect target changed");
      if (this.disposed) throw new Error("Effect controls disposed");
      const previous = this.current;
      // Never retarget an already queued gesture after a layer switch.
      const ack = await this.host.command(
        MODEL_COMMAND,
        json({
          layer: previous.layer,
          revision: previous.revision,
          documentId: previous.documentId,
          intent,
        }),
      );
      if (ack.value === undefined)
        throw new Error("Missing acknowledged model projection");
      const next = projection(ack.value);
      if (next.revision !== ack.revision)
        throw new Error("Projection/ack revision mismatch");
      await this.adopt(ack.value);
      if (
        (intent.type === "navigate" || intent.type === "reorder") &&
        next.focus
      )
        this.platform.focus(`${PANEL_ID}:effect:${next.focus}`);
      if (intent.type === "add") {
        if (next.focusLabel) this.platform.announce(`Added ${next.focusLabel}`);
      } else if (intent.type === "remove") {
        const effect = previous.rows.find(
          (r) => r.kind === "effect" && r.id === intent.id,
        ) as EffectHeader | undefined;
        if (effect) this.platform.announce(`Removed ${effect.label}`);
      } else if (intent.type === "paste")
        this.platform.announce(
          `Pasted ${Math.max(0, next.effectCount - previous.effectCount)} effects`,
        );
      else if (intent.type === "reorder" && intent.direction === -1) {
        const effect = previous.rows.find(
          (r) => r.kind === "effect" && r.id === intent.id,
        ) as EffectHeader | undefined;
        if (effect) this.platform.announce(`Moved ${effect.label} up`);
      }
      return next;
    });
  }
  send(intent: Intent) {
    void this.drive(intent).catch(() => {
      /* enqueue reports through platform */
    });
  }
  async add() {
    if (this.current.locked || this.current.target !== "Layer") return;
    const layer = this.current.layer;
    const document = this.current.documentId;
    const definition = await this.platform.effectMenu();
    if (
      definition &&
      layer === this.current.layer &&
      document === this.current.documentId
    )
      await this.drive({ type: "add", definition });
  }
  async menu(id: string) {
    const p = this.current;
    if (p.locked) return;
    const row = p.rows.find((r) => r.kind === "effect" && r.id === id) as
      | EffectHeader
      | undefined;
    if (!row) return;
    const multi = p.selectedCount > 1;
    const items: MenuItem[] = [
      { id: "copy", label: "Copy" },
      { id: "duplicate", label: "Duplicate Effect" },
      {
        id: "paste",
        label: p.clipboardCount > 1 ? "Paste Effects" : "Paste Effect",
        disabled: p.clipboardCount === 0,
        ...(p.clipboardCount === 0 ? { tooltip: "Copy an effect first" } : {}),
      },
      {
        id: "up",
        label: "Move Up",
        separator: true,
        disabled: row.ordinal === 0,
      },
      {
        id: "down",
        label: "Move Down",
        disabled: row.ordinal + 1 === row.effectCount,
      },
      {
        id: "reset",
        label: "Reset Effect",
        separator: true,
        disabled: row.missing,
      },
      { id: "remove", label: multi ? "Remove Effects" : "Remove Effect" },
    ];
    const choice = await this.platform.contextMenu(items);
    if (
      !choice ||
      this.current.layer !== p.layer ||
      this.current.documentId !== p.documentId ||
      items.find((i) => i.id === choice)?.disabled
    )
      return;
    if (choice === "up" || choice === "down")
      await this.drive({
        type: "reorder",
        id,
        direction: choice === "up" ? -1 : 1,
      });
    else if (["copy", "duplicate", "reset", "remove"].includes(choice))
      await this.drive({
        type: choice as "copy" | "duplicate" | "reset" | "remove",
        id,
      });
    else if (choice === "paste") await this.drive({ type: "paste" });
  }
  /** A parameter row's menu; Reset clears keys and expression (v1 B367). */
  async parameterMenu(id: string) {
    const p = this.current;
    const row = p.rows.find(
      (r) => (r.kind === "parameter" || r.kind === "point") && r.id === id,
    );
    // A point resets both of its numbers.
    const params =
      row?.kind === "parameter"
        ? [row.parameter]
        : row?.kind === "point"
          ? row.parameters
          : [];
    if (p.locked || !params.length || params.some((q) => q.readonly)) return;
    const choice = await this.platform.contextMenu([
      { id: "reset", label: "Reset" },
    ]);
    if (
      choice === "reset" &&
      this.current.layer === p.layer &&
      this.current.documentId === p.documentId
    )
      for (const q of params)
        await this.drive({ type: "parameter", id: q.id, action: "reset" });
  }
  async dispose() {
    this.disposed = true;
    await this.serial;
    for (const stop of this.stops) await stop();
    for (const binding of this.bindings)
      await this.host.disposeBinding(binding);
    this.bindings.clear();
    this.listeners.clear();
  }
}
