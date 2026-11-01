import { deepStrictEqual, ok } from "node:assert/strict";
import {
  createRoot,
  type Commit,
  type ComponentChild,
  type Json,
  type ListenerFlags,
  type SurfaceEvent,
} from "powermove-v4";

export interface SnapshotNode {
  id: number;
  type: string;
  props: Record<string, Json>;
  text: string;
  children: number[];
}
interface WireNode extends SnapshotNode {
  parent: number | null;
  listeners: Record<string, ListenerFlags>;
}
interface WireResult {
  accepted: boolean;
  revision: number;
  nodes: Record<string, WireNode>;
  diagnostics?: unknown[];
  error?: unknown;
}

/** Test host only. Extensions use public snapshots, never root.host or its DOM adapter. */
export class Evaluation {
  readonly commits: Commit[] = [];
  readonly wireResults: WireResult[] = [];
  readonly root;
  private listeners = new Map<number, Record<string, ListenerFlags>>();
  // Native input observations are out-of-band state. The JSON-lines Rust example
  // accepts only commits; this overlay models the native observation separately,
  // without manufacturing a JS op or consuming a surface revision.
  readonly nativeObservations = new Map<number, Json>();
  readonly timings: number[] = [];
  constructor(readonly wire?: string) {
    this.root = createRoot(
      async (commit) => {
        const result = this.wire
          ? this.replay([...this.commits, commit]).at(-1)!
          : { accepted: true, revision: commit.baseRevision + 1 };
        if (result.accepted) {
          this.commits.push(structuredClone(commit));
          for (const op of commit.ops) {
            if (op.op === "setProps" && "value" in op.patch)
              this.nativeObservations.delete(op.id);
            if (op.op === "setListeners")
              this.listeners.set(op.id, JSON.parse(JSON.stringify(op.flags)));
          }
        }
        if ("nodes" in result) this.wireResults.push(result as WireResult);
        return result;
      },
      () => {},
    );
  }
  replay(commits: Commit[], allowWebview = true): WireResult[] {
    ok(
      this.wire,
      "real Rust wire validator required for rejection/opt-in checks",
    );
    const child = Bun.spawnSync([this.wire], {
      stdin: new TextEncoder().encode(
        commits.map((commit) => JSON.stringify(commit)).join("\n") + "\n",
      ),
      env: { ...process.env, PM_SURFACE_WEBVIEW: allowWebview ? "1" : "0" },
    });
    deepStrictEqual(child.exitCode, 0, new TextDecoder().decode(child.stderr));
    const rows = new TextDecoder()
      .decode(child.stdout)
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as WireResult);
    deepStrictEqual(rows.length, commits.length, "one response per batch");
    return rows;
  }
  async render(view: ComponentChild) {
    const start = performance.now();
    const ack = await this.root.render(view);
    this.timings.push(performance.now() - start);
    if (ack) ok(ack.accepted, JSON.stringify(ack.error));
    this.compareNative();
  }
  async flush() {
    await this.root.flush();
    this.compareNative();
  }
  nodes(): SnapshotNode[] {
    return this.root.inspect().nodes as SnapshotNode[];
  }
  find(id: string) {
    return this.nodes().find((node) => node.props.id === id);
  }
  node(id: string | number): SnapshotNode {
    const node =
      typeof id === "number"
        ? this.nodes().find((node) => node.id === id)
        : this.find(id);
    ok(node, `missing node ${id}`);
    return node;
  }
  byLabel(label: string): SnapshotNode {
    const node =
      this.nodes().find((node) => node.props.label === label) ??
      this.nodes().find(
        (node) =>
          node.type === "button" &&
          node.children.some((id) => this.node(id).text === label),
      );
    ok(node, `missing label ${label}`);
    return node;
  }
  props(id: string, expected: Record<string, Json>) {
    for (const [key, value] of Object.entries(expected))
      deepStrictEqual(this.node(id).props[key], value, `${id}.${key}`);
  }
  assert(condition: unknown, message: string): asserts condition {
    ok(condition, message);
  }
  equal(actual: unknown, expected: unknown, message: string) {
    deepStrictEqual(actual, expected, message);
  }
  listener(id: number, type: string, flags: ListenerFlags) {
    deepStrictEqual(this.listeners.get(id)?.[type], flags);
    if (this.wire)
      deepStrictEqual(
        this.wireResults.at(-1)?.nodes[id].listeners[type],
        flags,
      );
  }
  async event(
    target: string | number,
    type: string,
    detail: Partial<
      Pick<SurfaceEvent, "key" | "value" | "shiftKey" | "altKey" | "pointerId">
    > = {},
  ) {
    const node = this.node(target);
    if (type === "input" && detail.value !== undefined)
      this.nativeObservations.set(node.id, detail.value);
    // A synthetic native planner walks the accepted snapshot, not the runtime host.
    const ancestors: SnapshotNode[] = [node];
    let parent: SnapshotNode | undefined;
    while (
      (parent = this.nodes().find((n) =>
        n.children.includes(ancestors.at(-1)!.id),
      ))
    )
      ancestors.push(parent);
    const deliveries: { id: number; capture: boolean }[] = [];
    let preventDefault = false;
    for (const capture of [true, false]) {
      for (const current of capture ? [...ancestors].reverse() : ancestors) {
        const flags = this.listeners.get(current.id)?.[type];
        if (!flags) continue;
        if (
          flags.preventDefault &&
          (!flags.keys.length || flags.keys.includes(detail.key ?? ""))
        )
          preventDefault = true;
        if (capture ? flags.capture : flags.bubble)
          deliveries.push({ id: current.id, capture });
      }
    }
    await this.root.dispatch(
      {
        revision: this.root.inspect().revision,
        target: node.id,
        preventDefault,
        deliveries,
      },
      type,
      detail,
    );
    await this.flush();
  }
  compareNative() {
    if (!this.wire) return;
    const result = this.wireResults.at(-1);
    ok(result?.accepted, "accepted native result");
    const projected = Object.values(result.nodes).map(
      ({ id, type, props, text, children }) => ({
        id,
        type,
        props,
        text,
        children,
      }),
    );
    for (const node of projected) {
      if (this.nativeObservations.has(node.id))
        node.props = {
          ...node.props,
          value: this.nativeObservations.get(node.id)!,
        };
    }
    deepStrictEqual(
      projected.sort((a, b) => a.id - b.id),
      this.nodes().sort((a, b) => a.id - b.id),
      "complete accepted snapshot including props and text",
    );
    deepStrictEqual(
      result.revision,
      this.root.inspect().revision,
      "revision matches",
    );
    for (const node of Object.values(result.nodes)) {
      deepStrictEqual(
        node.listeners,
        this.listeners.get(node.id) ?? {},
        "complete native listener map",
      );
      const parent = this.nodes().find((n) => n.children.includes(node.id));
      deepStrictEqual(node.parent, parent?.id ?? null, "native parent");
    }
  }
  async assertWebviewRequiresOptIn() {
    const result = this.replay(this.commits, false).at(-1)!;
    ok(!result.accepted, "webview rejected without host opt-in");
    deepStrictEqual(result.revision, 0);
    deepStrictEqual(Object.keys(result.nodes), ["0"]);
  }
  async rejectTail() {
    const before = this.replay(this.commits).at(-1)!;
    const next = Math.max(...this.nodes().map((n) => n.id)) + 1;
    const invalid: Commit = {
      baseRevision: before.revision,
      ops: [
        {
          op: "setProps",
          id: this.nodes().find((n) => n.id !== 0)!.id,
          patch: { label: "must roll back" },
        },
        { op: "create", id: next, type: "button" },
        { op: "insertBefore", parent: 0, id: next, before: null },
        { op: "insertBefore", parent: next, id: 0, before: null },
      ],
    };
    const rejected = this.replay([...this.commits, invalid]).at(-1)!;
    ok(!rejected.accepted, "invalid tail rejected by Rust");
    deepStrictEqual(rejected.nodes, before.nodes, "no valid leading op leaks");
    deepStrictEqual(rejected.revision, before.revision, "no revision consumed");
  }
  async dispose() {
    if (!this.root.inspect().fenced) await this.root.dispose();
  }
}
