import { render, options, type ComponentChild, type VNode } from "preact";
import { Host, EMPTY_STRING } from "./host";
import {
  json,
  type Ack,
  type EventPlan,
  type EventDetail,
  type InputObservation,
  type SnapshotNode,
  textRange,
  eventDetail,
  type NativeEvent,
  type SurfaceEvent,
  type Transport,
} from "./protocol";

const previousVNode = options.vnode;
options.vnode = (vnode: VNode) => {
  previousVNode?.(vnode);
  if (typeof vnode.type !== "string") return;
  const props: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(vnode.props)) {
    if (key === "children" || key === "ref" || key === "key") {
      props[key] = value;
      continue;
    }
    if (/^on[A-Za-z]/.test(key)) {
      if (value != null && typeof value !== "function")
        throw new TypeError(`${key} must be a handler`);
      props[key] = value;
    } else {
      if (key === "dangerouslySetInnerHTML")
        throw new TypeError("Native surfaces do not accept HTML");
      if (value !== undefined) json(value);
      // Prevent Preact's DOM property names/style/false-attribute rules from changing native data.
      const name = key === "value" || key.startsWith("pm:") ? key : `pm:${key}`;
      props[name] = value === "" ? EMPTY_STRING : value;
    }
  }
  vnode.props = props as typeof vnode.props;
};
let pending: (() => void)[] = [];
const roots = new Set<SurfaceRoot>();
let scheduled = false;
function schedule() {
  if (scheduled) return;
  scheduled = true;
  Promise.resolve().then(() => {
    scheduled = false;
    try {
      flushUpdates();
    } catch (error) {
      for (const root of roots) root.fail(error);
      return;
    }
    for (const root of roots)
      if (root.host.ops.length && root.host.poisoned === null) {
        root.flush().catch((error) => root.fail(error));
      }
  });
}
export function installScheduler() {
  options.debounceRendering = (callback) => {
    pending.push(callback);
    schedule();
  };
}
installScheduler();
/** Host drains at event, render, and refresh boundaries; no browser event loop required. */
export function flushUpdates() {
  let turns = 0;
  while (pending.length) {
    if (++turns > 100) throw new Error("Surface update loop");
    const current = pending;
    pending = [];
    for (const callback of current) callback();
  }
}
export interface RootOptions {
  generation?: number;
  /** Independent worker/native retirement; never a surface mutation. */
  retire?: (generation: number) => Promise<void>;
}
export interface RecoveryAdapter {
  reset(request: {
    generation: number;
    nextGeneration: number;
    state: import("./protocol").Json;
  }): Promise<{
    accepted: boolean;
    generation: number;
    revision: number;
    transport: Transport;
  }>;
}
export class SurfaceRoot {
  readonly host = new Host();
  private revision = 0;
  private chain: Promise<unknown> = Promise.resolve();
  private snapshot: SnapshotNode[] = [];
  private inputs = new Map<number, InputObservation>();
  private captures = new Map<number, number>();
  private observationSequence = 0;
  private disposal: Promise<void> | null = null;
  readonly generation: number;
  private disposed = false;
  private retiring = false;
  constructor(
    private readonly transport: Transport,
    private readonly onError: (error: unknown) => void = () => {},
    private readonly rootOptions: RootOptions = {},
  ) {
    this.generation = rootOptions.generation ?? 1;
    if (!Number.isSafeInteger(this.generation) || this.generation < 1)
      throw new RangeError("Invalid surface generation");
    installScheduler();
    roots.add(this);
  }
  render(vnode: ComponentChild): Promise<Ack | null> {
    this.check();
    try {
      render(vnode, this.host.root as unknown as HTMLElement);
    } catch (error) {
      this.fail(error);
      throw error;
    }
    return this.flush();
  }
  flush(): Promise<Ack | null> {
    return this.flushQueued();
  }
  private flushQueued(retirement = false): Promise<Ack | null> {
    this.check(retirement);
    try {
      flushUpdates();
    } catch (error) {
      this.fail(error);
      throw error;
    }
    const ops = this.host.take();
    if (!ops.length)
      return this.chain.then(() => {
        this.check(true);
        return null;
      });
    const job = this.chain.then(async () => {
      this.check(true);
      try {
        const ack = await this.transport({ baseRevision: this.revision, ops });
        if (!ack.accepted || ack.revision !== this.revision + 1)
          throw new Error(
            `Surface commit rejected: ${JSON.stringify(ack.error ?? ack)}`,
          );
        this.revision = ack.revision;
        this.accept(ops);

        return ack;
      } catch (error) {
        this.fail(error);
        throw error;
      }
    });
    // Keep rejection observable to the caller without creating unhandled chain rejections.
    this.chain = job.catch(() => undefined);
    return job;
  }
  setStyleRules(rules: import("./styles").StyleRule[]) {
    this.check();
    this.host.root.setAttribute("styleRules", json(rules));
    return this.flush();
  }
  /** Accepted props are immutable; nodes projects the latest native input value. */
  inspect() {
    const nodes = structuredClone(this.snapshot);
    for (const node of nodes) {
      const input = this.inputs.get(node.id);
      if (input) node.props.value = input.value;
    }
    return {
      revision: this.revision,
      generation: this.generation,
      nodes,
      acceptedNodes: structuredClone(this.snapshot),
      inputs: Object.fromEntries(structuredClone([...this.inputs])),
      fenced: this.host.poisoned !== null,
      disposed: this.disposed,
    };
  }
  observeInput(
    id: number,
    observation: Omit<InputObservation, "sequence">,
    revision = this.revision,
  ) {
    this.check();
    const node = this.snapshot.find((n) => n.id === id);
    if (
      revision !== this.revision ||
      !node ||
      !["input", "textarea"].includes(node.type)
    )
      throw new Error("Stale or non-input observation");
    if (
      typeof observation.value !== "string" ||
      typeof observation.composing !== "boolean"
    )
      throw new TypeError("Invalid input observation");
    const selection = textRange(
      observation.selection,
      observation.value.length,
    );
    const composition =
      observation.composition === null
        ? null
        : textRange(observation.composition, observation.value.length);
    if (!observation.composing && composition !== null)
      throw new Error("Composition range requires composing input");
    const input = {
      ...observation,
      selection,
      composition,
      sequence: ++this.observationSequence,
    };
    this.inputs.set(id, input);
    this.host.nodes.get(id)!.props.value = input.value;
  }
  capturePointer(pointer: number, id: number) {
    this.check();
    if (
      !Number.isSafeInteger(pointer) ||
      pointer < 0 ||
      !this.snapshot.some((n) => n.id === id)
    )
      throw new Error("Invalid pointer capture");
    this.captures.set(pointer, id);
  }
  releasePointer(pointer: number) {
    this.captures.delete(pointer);
  }
  /** Headless/native planner over accepted declarations; defaults decided before JS. */
  async drive(
    target: number,
    type: string,
    detail: EventDetail = {},
    revision = this.revision,
    generation = this.generation,
  ) {
    await this.chain;
    this.check();
    if (revision !== this.revision || generation !== this.generation)
      throw new Error("Stale surface drive");
    detail = eventDetail(detail);
    target =
      detail.pointerId === undefined
        ? target
        : (this.captures.get(detail.pointerId) ?? target);
    const path: number[] = [];
    let node = this.snapshot.find((n) => n.id === target);
    if (!node) throw new Error("Missing surface drive target");
    while (node) {
      path.push(node.id);
      const id: number = node.id;
      node = this.snapshot.find((n) => n.children.includes(id));
    }
    const deliveries: EventPlan["deliveries"] = [];
    let preventDefault = false;
    for (const capture of [true, false])
      for (const id of capture ? [...path].reverse() : path) {
        const flags = this.acceptedListeners.get(id)?.[type.toLowerCase()];
        if (!flags) continue;
        preventDefault ||=
          flags.preventDefault &&
          (!flags.keys.length || flags.keys.includes(detail.key ?? ""));
        if (capture ? flags.capture : flags.bubble)
          deliveries.push({ id, capture });
      }
    try {
      return await this.dispatch(
        { revision, target, preventDefault, deliveries },
        type,
        detail,
      );
    } finally {
      if (
        ["pointerup", "pointercancel"].includes(type.toLowerCase()) &&
        detail.pointerId !== undefined
      )
        this.releasePointer(detail.pointerId);
    }
  }
  dispatchNative(event: NativeEvent) {
    if (event.generation !== this.generation)
      return Promise.reject(new Error("Stale native event generation"));
    return this.dispatch(event.plan, event.type, event.detail);
  }
  async dispatch(plan: EventPlan, type: string, detail: EventDetail = {}) {
    await this.chain;
    this.check();
    if (plan.revision !== this.revision || !this.host.nodes.has(plan.target))
      throw new Error("Stale surface event");
    detail = eventDetail(detail);
    if (
      detail.value !== undefined &&
      ["input", "textarea"].includes(this.host.nodes.get(plan.target)!.type)
    ) {
      if (typeof detail.value !== "string")
        throw new TypeError("Native text observation must be a string");
      this.observeInput(
        plan.target,
        {
          value: detail.value,
          selection: detail.selection ?? [
            detail.value.length,
            detail.value.length,
          ],
          composing: detail.composing ?? false,
          composition: detail.composition ?? null,
        },
        plan.revision,
      );
    }
    let stoppedAt: number | null = null,
      immediate = false;
    const event: SurfaceEvent = {
      ...detail,
      type,
      target: plan.target,
      currentTarget: plan.target,
      defaultPrevented: plan.preventDefault,
      preventDefault() {
        if (event.defaultPrevented) return;
        throw new Error(
          "Declare preventDefault on the element before dispatch",
        );
      },
      stopPropagation() {
        stoppedAt = event.currentTarget;
      },
      stopImmediatePropagation() {
        stoppedAt = event.currentTarget;
        immediate = true;
      },
    };
    for (const delivery of plan.deliveries) {
      if (immediate || (stoppedAt !== null && stoppedAt !== delivery.id)) break;
      const node = this.host.nodes.get(delivery.id);
      if (!node) throw new Error("Event path contains a missing element");
      event.currentTarget = delivery.id;
      node.handlers
        .get(`${type.toLowerCase()}:${delivery.capture}`)
        ?.call(node, event);
    }
    return this.flush();
  }
  private acceptedListeners = new Map<
    number,
    Record<string, import("./protocol").ListenerFlags>
  >();
  private accept(ops: import("./protocol").Op[]) {
    const nodes = new Map(this.snapshot.map((n) => [n.id, n]));
    const parents = new Map<number, number>();
    for (const node of nodes.values())
      for (const child of node.children) parents.set(child, node.id);
    if (!nodes.has(0))
      nodes.set(0, { id: 0, type: "root", children: [], props: {}, text: "" });
    for (const op of ops) {
      if (op.op === "create")
        nodes.set(op.id, {
          id: op.id,
          type: op.type,
          children: [],
          props: {},
          text: "",
        });
      else if (op.op === "insertBefore") {
        const oldParent = parents.get(op.id);
        if (oldParent !== undefined) {
          const parent = nodes.get(oldParent)!;
          parent.children.splice(parent.children.indexOf(op.id), 1);
        }
        parents.set(op.id, op.parent);
        const children = nodes.get(op.parent)!.children;
        children.splice(
          op.before === null ? children.length : children.indexOf(op.before),
          0,
          op.id,
        );
      } else if (op.op === "remove") {
        const remove = (id: number) => {
          for (const child of nodes.get(id)?.children ?? []) remove(child);
          nodes.delete(id);
          this.inputs.delete(id);
          this.acceptedListeners.delete(id);
          for (const [pointer, target] of this.captures)
            if (target === id) this.captures.delete(pointer);
        };
        const parent = nodes.get(parents.get(op.id)!);
        if (parent) parent.children.splice(parent.children.indexOf(op.id), 1);
        remove(op.id);
      } else if (op.op === "setText") nodes.get(op.id)!.text = op.text;
      else if (op.op === "setListeners")
        this.acceptedListeners.set(op.id, structuredClone(op.flags));
      else {
        const node = nodes.get(op.id)!;
        for (const [key, value] of Object.entries(op.patch)) {
          if (value === null) delete node.props[key];
          else node.props[key] = structuredClone(value);
          if (key === "value") {
            const input = this.inputs.get(op.id);
            if (input && value !== input.value) {
              this.inputs.delete(op.id);
            }
          }
        }
      }
    }
    this.snapshot = [...nodes.values()].sort((a, b) => a.id - b.id);
  }
  dispose(): Promise<void> {
    if (this.disposal) return this.disposal;
    this.retiring = true;
    roots.delete(this);
    this.disposal = (async () => {
      try {
        await this.chain;
        if (this.host.poisoned === null) {
          render(null, this.host.root as unknown as HTMLElement);
          await this.flushQueued(true);
        }
      } finally {
        // Local unmount runs effect cleanups even when native mutation is fenced.
        try {
          render(null, this.host.root as unknown as HTMLElement);
        } finally {
          this.disposed = true;
          roots.delete(this);
          this.host.take();
          for (const node of this.host.nodes.values()) node.handlers.clear();
          this.host.nodes.clear();
          this.host.nodes.set(0, this.host.root);
          this.host.root.children = [];
          this.host.root.props = {};
          this.inputs.clear();
          this.captures.clear();
          this.acceptedListeners.clear();
          await this.rootOptions.retire?.(this.generation);
        }
      }
    })();
    return this.disposal;
  }
  /** Caller snapshots extension state before retirement; reset must acknowledge a fresh generation. */
  async recover(state: import("./protocol").Json, adapter: RecoveryAdapter) {
    const saved = json(state);
    const nextGeneration = this.generation + 1;
    if (!Number.isSafeInteger(nextGeneration))
      throw new RangeError("Generation exhausted");
    await this.dispose();
    const ack = await adapter.reset({
      generation: this.generation,
      nextGeneration,
      state: structuredClone(saved),
    });
    if (
      !ack.accepted ||
      ack.generation !== nextGeneration ||
      ack.revision !== 0
    )
      throw new Error("Recovery reset rejected");
    return {
      root: new SurfaceRoot(ack.transport, this.onError, {
        ...this.rootOptions,
        generation: nextGeneration,
      }),
      state: saved,
    };
  }
  /** Fences a failed root; accepted native state and inspector snapshot remain available. */
  fail(error: unknown) {
    if (this.host.poisoned !== null) return;
    this.host.poisoned = error ?? new Error("Surface root failed");
    roots.delete(this);
    this.onError(error);
  }
  private check(retirement = false) {
    if (this.disposed || (this.retiring && !retirement))
      throw new Error("Surface root disposed");
    if (this.host.poisoned !== null)
      throw new Error(
        "Surface root fenced; recover the realm from the last accepted native tree",
        { cause: this.host.poisoned },
      );
  }
}
export const createRoot = (
  transport: Transport,
  onError?: (error: unknown) => void,
  options?: RootOptions,
) => new SurfaceRoot(transport, onError, options);
