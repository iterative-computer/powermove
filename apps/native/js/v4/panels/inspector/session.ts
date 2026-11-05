import { HostClient, type HostQuery } from "../../src/host-api";
import type {
  Binding,
  InspectorModel,
  InspectorSnapshot,
  Intent,
  NativeRequest,
  NativeServices,
  ReadoutClock,
  Row,
} from "./contracts";
import { rowBindings } from "./contracts";

/** Worker-owned lifecycle. Only acknowledged host bindings are usable. */
export class InspectorSession {
  private stops: (() => Promise<void>)[] = [];
  private bindings = new Map<string, Binding>();
  private listeners = new Set<() => void>();
  private alive = true;
  private serial: Promise<void> = Promise.resolve();
  private cancelFrame?: () => void;
  private snapshot: InspectorSnapshot = {
    target: { kind: "no-project" },
    sections: [],
    playing: false,
    time: 0,
  };
  private started = false;
  constructor(
    readonly host: HostClient,
    private readonly model: InspectorModel,
    private readonly native: NativeServices,
    private readonly clock: ReadoutClock,
    readonly onError: (error: unknown) => void,
  ) {}
  inspect() {
    return this.snapshot;
  }
  subscribe(callback: () => void) {
    this.listeners.add(callback);
    return () => {
      this.listeners.delete(callback);
    };
  }
  ready(handle: string) {
    return this.bindings.has(handle);
  }
  private notify() {
    if (this.alive) for (const listener of this.listeners) listener();
  }
  private queue(job: () => Promise<void>) {
    const next = this.serial.then(job);
    this.serial = next.catch((error) => {
      this.onError(error);
    });
    return next;
  }
  async start() {
    if (this.started || !this.alive)
      throw new Error("Inspector session already started/retired");
    this.started = true;
    try {
      for (const query of [
        "project",
        "selection",
        "time",
        "evaluated",
      ] as HostQuery[]) {
        if (!this.alive) return;
        const stop = await this.host.subscribe(
          `inspector:${query}`,
          query,
          (value) => {
            if (!this.alive) return;
            this.model.update(query, value);
            const next = this.model.inspect();
            if (query === "time" || query === "evaluated") {
              if (next.playing) {
                this.cancelFrame ??= this.clock.schedule(() => {
                  this.cancelFrame = undefined;
                  if (this.alive) this.refresh();
                });
                return;
              }
            }
            // Pausing bypasses the display-clock delay: exact stopped value now.
            this.cancelFrame?.();
            this.cancelFrame = undefined;
            this.refresh();
          },
        );
        if (this.alive) this.stops.push(stop);
        else await stop();
      }
      await this.settled();
    } catch (error) {
      await this.dispose();
      throw error;
    }
  }
  private refresh() {
    this.snapshot = structuredClone(this.model.inspect());
    this.notify();
    void this.queue(async () => {
      if (!this.alive) return;
      const wanted = new Map<string, Binding>();
      for (const section of this.snapshot.sections) {
        if (section.collapsed) continue;
        for (const row of section.rows) {
          for (const binding of rowBindings(row)) {
            const duplicate = wanted.get(binding.handle);
            if (
              duplicate &&
              JSON.stringify(duplicate.channels) !==
                JSON.stringify(binding.channels)
            )
              throw new Error("Conflicting inspector binding handle");
            wanted.set(binding.handle, binding);
          }
        }
      }
      for (const [handle, old] of this.bindings) {
        const next = wanted.get(handle);
        if (
          !next ||
          JSON.stringify(next.channels) !== JSON.stringify(old.channels)
        ) {
          this.bindings.delete(handle);
          // Cancellation/disposal is native and precedes new registration.
          await this.host.disposeBinding(handle);
        }
      }
      for (const [handle, binding] of wanted) {
        if (!this.alive) return;
        if (!this.bindings.has(handle)) {
          await this.host.registerBinding(handle, binding.channels);
          if (!this.alive) {
            await this.host.disposeBinding(handle);
            return;
          }
          this.bindings.set(handle, binding);
        }
      }
      this.notify();
    }).catch(() => {}); // queue reports once; no unhandled worker rejection
  }
  /** UI callbacks report asynchronous failures through the host diagnostics path. */
  send(intent: Intent) {
    void this.dispatch(intent).catch(this.onError);
  }
  async dispatch(intent: Intent) {
    if (!this.alive) throw new Error("Inspector session retired");
    // Stale UI events cannot mutate a newly selected layer through old row IDs.
    if ("row" in intent) {
      const row = this.row(intent.row);
      if (!row || row.disabled || row.locked) return false;
    }
    if (intent.type === "drop-effect") {
      const target = this.snapshot.target;
      if (
        target.kind !== "layers" ||
        target.first.id !== intent.target ||
        target.first.locked ||
        !target.first.acceptsEffects
      )
        return false;
    }
    if (
      intent.type === "align" &&
      (this.snapshot.target.kind !== "layers" ||
        this.snapshot.target.first.audio ||
        this.snapshot.target.first.locked)
    )
      return false;
    const operation = this.model.plan(intent);
    if (!operation) return false;
    if (operation.type === "edit")
      await this.host.edit(operation.label, operation.commands);
    else await this.host.command(operation.command, operation.args);
    return true;
  }
  row(id: string): Row | undefined {
    for (const section of this.snapshot.sections) {
      if (!section.collapsed) {
        const row = section.rows.find((row) => row.id === id);
        if (row) return row;
      }
    }
  }
  nativeRequest(request: NativeRequest) {
    if (this.alive) void this.native.request(request).catch(this.onError);
  }
  async settled() {
    await this.serial;
  }
  async dispose() {
    if (!this.alive) return;
    this.alive = false;
    this.cancelFrame?.();
    this.cancelFrame = undefined;
    this.listeners.clear();
    await this.settled();
    const errors: unknown[] = [];
    for (const stop of this.stops.splice(0).reverse()) {
      try {
        await stop();
      } catch (error) {
        errors.push(error);
      }
    }
    for (const handle of this.bindings.keys()) {
      try {
        await this.host.disposeBinding(handle);
      } catch (error) {
        errors.push(error);
      }
    }
    this.bindings.clear();
    // HostClient may be shared. Independent realm retirement remains the owner’s job.
    if (errors.length)
      this.onError(new AggregateError(errors, "Inspector cleanup failed"));
  }
}
