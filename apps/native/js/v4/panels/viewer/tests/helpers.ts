import { expect } from "bun:test";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  createRoot,
  type Commit,
  type HostAck,
  type HostRequest,
} from "../../../src";
import type { ViewerSnapshot } from "../contract";

export function fixture(): ViewerSnapshot {
  return {
    model: {
      ticket: "1",
      project: "p",
      composition: "c",
      name: "Launch Film",
      fit: true,
      shownZoom: 0.54,
      quality: "auto",
      effectiveQuality: 0.5,
      canvasLost: false,
      showControls: true,
      empty: false,
      selectionCount: 2,
    },
    host: {
      presentation: "Docked",
      frame: { resource: "synthetic:last-good", loading: false, failed: false },
      source: null,
      interaction: "idle",
    },
  };
}
export function recorder() {
  const commits: Commit[] = [];
  const root = createRoot(async (c) => {
    commits.push(structuredClone(c));
    return { accepted: true, revision: c.baseRevision + 1 };
  });
  return { root, commits };
}
let built = false;
export function binaries() {
  const workspace = fileURLToPath(
    new URL("../../../../../Cargo.toml", import.meta.url),
  );
  const target = resolve(
    process.env.CARGO_TARGET_DIR ??
      fileURLToPath(new URL("../../../../../target", import.meta.url)),
  );
  if (!built) {
    for (const args of [
      [
        "cargo",
        "build",
        "--locked",
        "--manifest-path",
        workspace,
        "--target-dir",
        target,
        "-p",
        "pm-panel-viewerchrome",
        "--examples",
      ],
    ]) {
      const result = Bun.spawnSync(args, { stdout: "pipe", stderr: "pipe" });
      if (result.exitCode) throw new Error(result.stderr.toString());
    }
    built = true;
  }
  return {
    ui: join(target, "debug/examples/v4_panel_wire"),
    model: join(target, "debug/examples/v4_wire"),
  };
}
export function replay(commits: Commit[]) {
  const result = Bun.spawnSync([binaries().ui], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(result.exitCode, result.stderr.toString()).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
export function ack(r: HostRequest, value?: HostAck["value"]): HostAck {
  return { ...r, revision: r.expectedRevision, accepted: true, value };
}
export async function modelWorker() {
  const proc = Bun.spawn([binaries().model], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const reader = proc.stdout.pipeThrough(new TextDecoderStream()).getReader();
  let buffer = "";
  async function line(): Promise<any> {
    while (!buffer.includes("\n")) {
      const next = await reader.read();
      if (next.done)
        throw new Error(
          `Model wire stopped: ${await new Response(proc.stderr).text()}`,
        );
      buffer += next.value;
    }
    const at = buffer.indexOf("\n");
    const result = JSON.parse(buffer.slice(0, at));
    buffer = buffer.slice(at + 1);
    return result;
  }
  const model = await line();
  return {
    model,
    async request(value: unknown) {
      proc.stdin.write(JSON.stringify(value) + "\n");
      proc.stdin.flush();
      return line();
    },
    async close() {
      proc.stdin.end();
      await proc.exited;
      reader.releaseLock();
    },
  };
}
