import { beforeAll, expect } from "bun:test";
import { fileURLToPath } from "node:url";
import {
  HostClient,
  createRoot,
  type Commit,
  type EventDetail,
  type SurfaceRoot,
  type Json,
} from "../../../src";
import { EffectsController } from "../controller";
import type { Platform, Projection } from "../contract";
export function platform() {
  const focus: string[] = [],
    announcements: string[] = [],
    errors: unknown[] = [];
  const api: Platform = {
    effectMenu: async () => null,
    contextMenu: async () => null,
    focus: (id) => {
      focus.push(id);
    },
    announce: (message) => {
      announcements.push(message);
    },
    report: (error) => {
      errors.push(error);
    },
  };
  return { api, focus, announcements, errors };
}
export async function fixture(value: Projection) {
  const kit = platform();
  const requests: unknown[] = [];
  const host = new HostClient(
    { realm: "effects", generation: 1, project: "P" },
    async (request) => {
      requests.push(request.operation);
      return {
        ...request,
        accepted: true,
        revision: value.revision,
        value: value as unknown as Json,
      };
    },
  );
  const controller = new EffectsController(host, kit.api, "effects");
  await controller.start();
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits, controller, kit, requests };
}
export function nodes(root: SurfaceRoot) {
  return [...root.host.nodes.values()];
}
export function find(
  root: SurfaceRoot,
  predicate: (props: Record<string, Json>) => boolean,
) {
  const node = nodes(root).find((n) => predicate(n.props));
  if (!node) throw new Error("Fixture node absent");
  return node;
}
export async function event(
  root: SurfaceRoot,
  id: number,
  type: string,
  detail: EventDetail = {},
  ancestors: number[] = [],
) {
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: id,
      preventDefault: true,
      deliveries: [
        ...ancestors.map((id) => ({ id, capture: true })),
        { id, capture: true },
        { id, capture: false },
        ...ancestors.reverse().map((id) => ({ id, capture: false })),
      ],
    },
    type,
    detail,
  );
  await root.flush();
}
let target: string | undefined;
export function buildNative() {
  if (target) return target;
  const manifest = fileURLToPath(
    new URL("../native/Cargo.toml", import.meta.url),
  );
  const build = Bun.spawnSync(
    [
      "cargo",
      "build",
      "--locked",
      "--manifest-path",
      manifest,
      "-p",
      "pm-effect-controls-wire",
      "--bins",
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  if (build.exitCode !== 0) throw new Error(build.stderr.toString());
  const metadata = Bun.spawnSync(
    [
      "cargo",
      "metadata",
      "--locked",
      "--no-deps",
      "--format-version",
      "1",
      "--manifest-path",
      manifest,
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  if (metadata.exitCode !== 0) throw new Error(metadata.stderr.toString());
  target = JSON.parse(metadata.stdout.toString()).target_directory as string;
  return target;
}
// Native compilation is setup, outside the UI interaction timeout.
beforeAll(() => {
  buildNative();
}, 120_000);
export function wire(commits: Commit[]) {
  const exe = `${buildNative()}/debug/effect-controls-wire`;
  const output = Bun.spawnSync([exe], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(output.exitCode).toBe(0);
  const results = output.stdout
    .toString()
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l));
  for (const result of results) {
    expect(result.accepted).toBe(true);
    expect(result.wireDiagnostics).toEqual([]);
  }
  return results.at(-1);
}
