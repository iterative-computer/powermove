import { inspectorWire as binary } from "./setup";
import {
  createRoot,
  type Commit,
  type EventDetail,
  type Json,
} from "../../../src";
import type { ComponentChild } from "preact";

export interface ProbeNode {
  id: number;
  type: string;
  props: Record<string, Json>;
  text: string;
  children: number[];
  parent: number | null;
  listeners: Record<string, unknown>;
}
export interface ProbeResult {
  accepted: boolean;
  error?: string;
  revision: number;
  nodes: Record<string, ProbeNode>;
  diagnostics: unknown[];
  wireDiagnostics: unknown[];
  focusOrder: number[];
  accessibility: {
    id: number;
    role: string;
    label: string | null;
    value: string | null;
    toggled: string | null;
    expanded: boolean | null;
    disabled: boolean;
    increment: boolean;
    decrement: boolean;
    step: number | null;
  }[];
  edits: { type: string; binding: string; value?: Json }[];
}
export function replay(lines: Json[]): ProbeResult[] {
  const child = Bun.spawnSync([binary], {
    stdin: new TextEncoder().encode(
      lines.map((line) => JSON.stringify(line)).join("\n") + "\n",
    ),
  });
  if (child.exitCode !== 0)
    throw new Error(new TextDecoder().decode(child.stderr));
  return new TextDecoder()
    .decode(child.stdout)
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
export class Probe {
  readonly commits: Commit[] = [];
  result!: ProbeResult;
  readonly root = createRoot(async (commit) => {
    const results = replay([...this.commits, commit] as unknown as Json[]);
    this.result = results.at(-1)!;
    if (this.result.accepted) this.commits.push(structuredClone(commit));
    return {
      accepted: this.result.accepted,
      revision: this.result.revision,
      error: this.result.error,
    };
  });
  async render(view: ComponentChild) {
    await this.root.render(view);
    if (!this.result.accepted) throw new Error(this.result.error);
  }
  async flush() {
    await this.root.flush();
  }
  node(id: string) {
    const node = Object.values(this.result.nodes).find(
      (node) => node.props.id === id,
    );
    if (!node) throw new Error(`Missing node ${id}`);
    return node;
  }
  named(label: string) {
    const node = Object.values(this.result.nodes).find(
      (node) => node.props.label === label && node.type === "button",
    );
    if (!node) throw new Error(`Missing button ${label}`);
    return node;
  }
  async drive(id: string | number, type: string, detail: EventDetail = {}) {
    const target = typeof id === "number" ? id : this.node(id).id;
    await this.root.drive(target, type, detail);
    await this.flush();
  }
  native(...requests: Json[]) {
    return replay([...(this.commits as unknown as Json[]), ...requests]).slice(
      this.commits.length,
    );
  }
  async dispose() {
    await this.root.dispose();
  }
}
