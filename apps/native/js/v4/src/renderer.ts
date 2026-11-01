import { render, options, type ComponentChild, type VNode } from "preact";
import { Host, EMPTY_STRING } from "./host";
import {
  json,
  type Ack,
  type EventPlan,
  type Json,
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
export class SurfaceRoot {
  readonly host = new Host();
  private revision = 0;
  private chain: Promise<unknown> = Promise.resolve();
  private snapshot: unknown[] = [];
  private disposed = false;
  constructor(
    private readonly transport: Transport,
    private readonly onError: (error: unknown) => void = () => {},
  ) {
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
    this.check();
    try {
      flushUpdates();
    } catch (error) {
      this.fail(error);
      throw error;
    }
    const ops = this.host.take();
    if (!ops.length)
      return this.chain.then(() => {
        this.check();
        return null;
      });
    const snapshot = [...this.host.nodes.values()].map((node) => ({
      id: node.id,
      type: node.type,
      children: node.children.map((c) => c.id),
      props: { ...node.props },
      text: node.data,
    }));
    const job = this.chain.then(async () => {
      this.check();
      try {
        const ack = await this.transport({ baseRevision: this.revision, ops });
        if (!ack.accepted || ack.revision !== this.revision + 1)
          throw new Error(
            `Surface commit rejected: ${JSON.stringify(ack.error ?? ack)}`,
          );
        this.revision = ack.revision;
        this.snapshot = snapshot;
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
    this.host.ops.push({
      op: "setProps",
      id: 0,
      patch: { styleRules: json(rules) },
    });
    return this.flush();
  }
  inspect() {
    return {
      revision: this.revision,
      nodes: structuredClone(this.snapshot),
      fenced: this.host.poisoned !== null,
    };
  }
  async dispatch(
    plan: EventPlan,
    type: string,
    detail: {
      key?: string;
      value?: Json;
      shiftKey?: boolean;
      altKey?: boolean;
      pointerId?: number;
    } = {},
  ) {
    await this.chain;
    this.check();
    if (plan.revision !== this.revision || !this.host.nodes.has(plan.target))
      throw new Error("Stale surface event");
    if (detail.value !== undefined) {
      const target = this.host.nodes.get(plan.target)!;
      // The virtual host observes the native value before JS runs (controlled input rule).
      if (target.type === "input" || target.type === "textarea")
        target.props.value = detail.value;
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
  async dispose() {
    await this.render(null);
    this.disposed = true;
    roots.delete(this);
  }
  /** Fences a failed root; accepted native state and inspector snapshot remain available. */
  fail(error: unknown) {
    if (this.host.poisoned !== null) return;
    this.host.poisoned = error;
    roots.delete(this);
    this.onError(error);
  }
  private check() {
    if (this.disposed) throw new Error("Surface root disposed");
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
) => new SurfaceRoot(transport, onError);
