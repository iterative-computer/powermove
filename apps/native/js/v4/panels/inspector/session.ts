import { HostClient, type HostQuery } from "../../src/host-api";
import type { Json } from "../../src/protocol";
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
import { rowBindings, sectionRows } from "./contracts";

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
  /** The latest refresh asked for; an older one never publishes over it. */
  private refreshes = 0;
  private refresh() {
    const next = structuredClone(this.model.inspect());
    const seq = ++this.refreshes;
    const wanted = new Map<string, Binding>();
    for (const section of next.sections) {
      if (section.collapsed) continue;
      for (const row of sectionRows(section)) {
        for (const binding of rowBindings(row)) {
          const duplicate = wanted.get(binding.handle);
          if (
            duplicate &&
            JSON.stringify(duplicate.channels) !==
              JSON.stringify(binding.channels)
          ) {
            this.onError(new Error("Conflicting inspector binding handle"));
            return;
          }
          wanted.set(binding.handle, binding);
        }
      }
    }
    const bound = [...wanted].every(
      ([handle, binding]) =>
        JSON.stringify(this.bindings.get(handle)?.channels) ===
        JSON.stringify(binding.channels),
    );
    // What a shown state no longer uses lets go of its bindings after it is
    // gone.
    const release = async () => {
      for (const handle of [...this.bindings.keys()]) {
        if (seq !== this.refreshes) return;
        if (!wanted.has(handle)) {
          this.bindings.delete(handle);
          await this.host.disposeBinding(handle);
        }
      }
    };
    if (bound) {
      // Nothing new to bind (a value, the time): shown at once.
      this.snapshot = next;
      this.notify();
      if ([...this.bindings.keys()].some((h) => !wanted.has(h)))
        void this.queue(release).catch(() => {});
      return;
    }
    void this.queue(async () => {
      if (!this.alive) return;
      // A handle whose channels change is disposed before it is registered
      // again (cancellation/disposal is native and precedes registration).
      for (const [handle, old] of this.bindings) {
        const again = wanted.get(handle);
        if (
          again &&
          JSON.stringify(again.channels) !== JSON.stringify(old.channels)
        ) {
          this.bindings.delete(handle);
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
      // A new state (another layer, a section opened) shows once its fields
      // are bound: never a frame of disabled fields while they register.
      if (seq !== this.refreshes) return;
      this.snapshot = next;
      this.notify();
      await release();
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
      (intent.type === "align" || intent.type === "distribute") &&
      (this.snapshot.target.kind !== "layers" ||
        this.snapshot.target.first.audio ||
        this.snapshot.target.first.locked)
    )
      return false;
    // Distribute spaces three or more layers; the outermost stay.
    if (
      intent.type === "distribute" &&
      (this.snapshot.target.kind !== "layers" || this.snapshot.target.count < 3)
    )
      return false;
    const operation = this.model.plan(intent);
    if (!operation) return false;
    if (operation.type === "edit")
      await this.host.edit(operation.label, operation.commands);
    else {
      const ack = (await this.host.command(
        operation.command,
        operation.args,
      )) as { value?: unknown } | undefined;
      // A disclosure changes no document, so no publication follows: the
      // reply is the new projection. Apply it, or the group never opens.
      const value = ack?.value;
      if (
        intent.type === "collapse" &&
        this.alive &&
        value &&
        typeof value === "object" &&
        "sections" in value
      ) {
        this.model.update("evaluated", value as Json);
        this.refresh();
      }
    }
    return true;
  }
  row(id: string): Row | undefined {
    for (const section of this.snapshot.sections) {
      if (!section.collapsed) {
        const row = sectionRows(section).find((row) => row.id === id);
        if (row) return row;
      }
    }
  }
  nativeRequest(request: NativeRequest) {
    if (!this.alive) return;
    // A menu can outlive its document/selection. Keep the originating lease,
    // and never apply its result to the next publication's similarly named row.
    const target = this.snapshot.target;
    const key = "key" in target ? target.key : undefined;
    void this.native
      .request(request)
      .then(async (choice) => {
        if (!this.alive || typeof choice !== "string") return;
        const current = this.snapshot.target;
        if (!("key" in current) || current.key !== key) return;
        if (request.type === "add-effect-menu" && current.kind === "layers") {
          await this.dispatch({
            type: "drop-effect",
            effect: choice,
            target: request.target,
          });
        } else if (
          request.type === "font-menu" ||
          request.type === "fill-popover"
        ) {
          // One chosen value, one edit. A cancelled panel replies with null.
          await this.dispatch({
            type: "set-value",
            row: request.row,
            value: choice,
          });
        } else if (request.type === "animator-menu") {
          await this.dispatch({ type: "add-animator", preset: choice });
        } else if (request.type === "row-menu") {
          const item = request.items.find(
            (item) => item.action === choice && item.enabled,
          );
          if (item)
            await this.dispatch({
              type: "row-action",
              row: request.row,
              action: item.action,
            });
        }
      })
      .catch(this.onError);
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
