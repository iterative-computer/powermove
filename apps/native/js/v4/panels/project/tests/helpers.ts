import { binaries } from "./build-model";
import { expect } from "bun:test";
import {
  createRoot,
  HostClient,
  type Commit,
  type EventDetail,
  type Json,
  type SurfaceRoot,
} from "../../../src";
import { ProjectService } from "../service";
import type {
  Action,
  Availability,
  Projection,
  Reply,
  Stamp,
} from "../contract";
import type { NativeServices } from "../index";
export const native: NativeServices = {
  searchBinding: "test:search",
  renameBinding: "test:rename",
  activatePanel() {},
  menu() {},
};
export const layout = { columns: 2, rowHeight: 160 }; // Synthetic host geometry, never production defaults.
export function recorder() {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits };
}
export function replay(
  commits: Commit[],
  projection: Projection | null = null,
) {
  const binary = process.env.PM_PROJECT_INSPECT_WIRE ?? binaries().inspect;
  if (!binary)
    throw new Error("Build project-inspect-wire; see project README");
  const result = Bun.spawnSync([binary], {
    stdin: new TextEncoder().encode(
      commits
        .map((commit) => JSON.stringify({ commit, projection }))
        .join("\n") + "\n",
    ),
  });
  expect(result.exitCode).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l));
}
export async function event(
  root: SurfaceRoot,
  id: number,
  type: string,
  detail: EventDetail = {},
) {
  // Real root performs revision checks and capture/bubble. Derive ancestry from
  // the accepted op tree rather than directly calling component closures.
  const nodes = root.inspect().nodes;
  const path = [id];
  let at = id;
  while (true) {
    const parent = nodes.find((n) => n.children.includes(at));
    if (!parent) break;
    path.push(parent.id);
    at = parent.id;
  }
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: id,
      preventDefault: true,
      deliveries: path.map((id) => ({ id, capture: false })),
    },
    type,
    detail,
  );
}
export function find(root: SurfaceRoot, label: string) {
  return root.inspect().nodes.find((n) => n.props.label === label)!;
}
export function card(root: SurfaceRoot, id: string) {
  return root
    .inspect()
    .nodes.find(
      (n) => (n.props.attr as { item?: { Asset: string } })?.item?.Asset === id,
    )!;
}
export function treeSnapshot(
  nodes: Record<
    string,
    {
      id: number;
      type: string;
      props: object;
      children: number[];
      text: string;
    }
  >,
) {
  const build = (id: number): unknown => {
    const n = nodes[id];
    return [n.type, n.props, n.text, n.children.map(build)];
  };
  // Preact's useId counter is process-global. Normalize only generated IDs.
  return JSON.parse(
    JSON.stringify(build(0)).replace(/:P[0-9a-z]+:/gi, ":AUTO:"),
  );
}
export async function model(
  project: unknown,
  runtime: Record<string, Availability> = {},
) {
  const binary = process.env.PM_PROJECT_MODEL_HOST ?? binaries().model;
  if (!binary) throw new Error("Build project-model-host; see project README");
  const child = Bun.spawn([binary], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "inherit",
  });
  const reader = child.stdout.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  async function send(value: unknown): Promise<any> {
    child.stdin.write(JSON.stringify(value) + "\n");
    child.stdin.flush();
    while (!buffer.includes("\n")) {
      const part = await reader.read();
      if (part.done) throw new Error("model host exited");
      buffer += decoder.decode(part.value, { stream: true });
    }
    const end = buffer.indexOf("\n");
    const line = buffer.slice(0, end);
    buffer = buffer.slice(end + 1);
    return JSON.parse(line);
  }
  const initial = await send({ initialize: project, runtime });
  if (initial.error) throw new Error(initial.error);
  let view: Projection = initial.view;
  const requests: Action[] = [];
  const host = new HostClient(
    { realm: "project-panel-test", generation: 1, project: "P" },
    async (request) => {
      let value: Json | undefined;
      let error: string | undefined;
      if (request.operation.type === "subscribe")
        value = { stamp: view.page.stamp } as unknown as Json;
      else if (request.operation.type === "command") {
        const result = await send(request.operation.args);
        error = result.error;
        if (!error) {
          view = result.reply.view;
          value = result.reply;
          requests.push(
            (request.operation.args as unknown as { action: Action }).action,
          );
        }
      }
      return {
        ...request,
        accepted: !error,
        revision: Math.max(view.page.stamp.revision, request.expectedRevision),
        value,
        error,
      };
    },
  );
  const service = new ProjectService(host, {
    command: "test.registered.media",
    subscription: "test.media.project",
    projectStamp: (value) => (value as unknown as { stamp: Stamp }).stamp,
  });
  await service.page(0, 10);
  await service.start();
  return {
    child,
    service,
    host,
    requests,
    send,
    get view() {
      return view;
    },
    async close() {
      await service.dispose();
      child.stdin.end();
      await child.exited;
    },
    async action(action: Action): Promise<Reply> {
      const result = await service.send(action);
      if (!result) throw new Error("missing model reply");
      return result;
    },
  };
}
