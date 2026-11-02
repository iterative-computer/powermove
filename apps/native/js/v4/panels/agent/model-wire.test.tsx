import { beforeAll, test, expect } from "bun:test";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { HostClient, type HostAck } from "../../src/host-api";
import { createRoot, type Commit, type Json } from "../../src";
import { HostAgentModel } from "./host";
import type { Publication } from "./contract";
import { AgentPanel } from "./view";
import { fixture } from "./fixtures";

let executable: string;
function wire() {
  if (!executable) {
    const manifest = fileURLToPath(
      new URL("../../../../Cargo.toml", import.meta.url),
    );
    const target = resolve(
      process.env.CARGO_TARGET_DIR ??
        fileURLToPath(new URL("../../../../target/", import.meta.url)),
    );
    const result = Bun.spawnSync([
      "cargo",
      "build",
      "--locked",
      "--manifest-path",
      manifest,
      "--target-dir",
      target,
      "-p",
      "pm-panel-agentpanel",
      "--example",
      "agent-panel-wire",
    ]);
    if (result.exitCode !== 0) throw new Error(result.stderr.toString());
    executable = join(target, "debug/examples/agent-panel-wire");
  }
  return executable;
}
// A cold model build is setup work, outside five-second UI interactions.
beforeAll(() => {
  wire();
}, 120_000);

async function nativeModel() {
  let publish = (p: Publication) => {};
  const history: Json[] = [{ native: fixture().native as unknown as Json }];
  let pendingEffects = 0;
  const client = new HostClient(
    { realm: "synthetic-agent", project: "synthetic-project", generation: 1 },
    async (request) => {
      history.push({ request: request as unknown as Json });
      const response = Bun.spawnSync([wire()], {
        stdin: new TextEncoder().encode(
          history.map((v) => JSON.stringify(v)).join("\n") + "\n",
        ),
      });
      expect(response.exitCode).toBe(0);
      const result = JSON.parse(
        response.stdout.toString().trim().split("\n").at(-1)!,
      );
      if (result.error) throw new Error(result.error);
      pendingEffects = result.pendingEffects;
      const ack = result.ack as HostAck;
      if (
        ack.accepted &&
        request.operation.type === "command" &&
        request.operation.command === "fixture.dispatch"
      )
        publish(ack.value as unknown as Publication);
      return ack;
    },
  );
  const model = await HostAgentModel.connect(client, {
    read: "fixture.read",
    dispatch: "fixture.dispatch",
    subscribe: (callback) => {
      publish = callback;
      return () => {
        publish = () => {};
      };
    },
  });
  return {
    model,
    get pendingEffects() {
      return pendingEffects;
    },
  };
}
test("actual Rust model through HostRouter/HostDataService drives Send, Stop, consent and the six-op panel", async () => {
  const f = await nativeModel();
  const commits: Commit[] = [];
  const errors: unknown[] = [];
  const root = createRoot(async (commit) => {
    commits.push(commit);
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  await root.render(
    <AgentPanel model={f.model} onError={(e) => errors.push(e)} />,
  );
  await f.model.dispatch({
    type: "draft",
    text: "Authored 日本🙂\n",
    selection: [14, 14],
  });
  await root.flush();
  expect(f.model.inspect().snapshot.draft.text).toBe("Authored 日本🙂\n");
  const send = root.inspect().nodes.find((n) => n.props.id === "agent-send")!;
  await root.drive(send.id, "click");
  // Async host traffic is deliberately outside root.drive's JS callback flush.
  await f.model.flush();
  await root.flush();
  expect(f.model.inspect().snapshot.runId).toBe("run-3");
  expect(f.pendingEffects).toBe(1);
  expect(f.model.inspect().snapshot.rows.items[0].value.text).toBe(
    "Authored 日本🙂\n",
  );
  const stop = root.inspect().nodes.find((n) => n.props.id === "agent-stop")!;
  await root.drive(stop.id, "click");
  await f.model.flush();
  await root.flush();
  expect(f.model.inspect().snapshot.stopping).toBe(true);
  expect(f.pendingEffects).toBe(2);
  await f.model.dispatch({ type: "setAccess", access: "full" });
  expect(f.model.inspect().snapshot.settings.access).toBe("editor");
  expect(f.pendingEffects).toBe(3);
  const validated = Bun.spawnSync([process.env.PM_UI_KIT_WIRE!], {
    stdin: new TextEncoder().encode(
      commits.map((c) => JSON.stringify(c)).join("\n") + "\n",
    ),
  });
  expect(validated.exitCode).toBe(0);
  for (const line of validated.stdout.toString().trim().split("\n")) {
    const row = JSON.parse(line);
    expect(row.accepted).toBe(true);
    expect(row.diagnostics).toEqual([]);
    expect(row.wireDiagnostics).toEqual([]);
  }
  expect(errors).toEqual([]);
  await root.dispose();
  f.model.dispose();
});
test("real model key dispatch preserves IME, autorepeat, newline, slash and local select-all behavior", async () => {
  const { model } = await nativeModel();
  await model.dispatch({ type: "draft", text: "Keep", selection: [4, 4] });
  for (const key of [
    { key: "Enter", composing: true },
    { key: "Enter", repeat: true },
  ]) {
    await model.dispatch({ type: "key", key });
    expect(model.inspect().snapshot.runId).toBeNull();
  }
  await model.dispatch({ type: "key", key: { key: "Enter", shiftKey: true } });
  expect(model.inspect().snapshot.draft.text).toBe("Keep\n");
  await model.dispatch({ type: "key", key: { key: "a", metaKey: true } });
  expect(model.inspect().snapshot.draft.selection).toEqual([0, 5]);
  await model.dispatch({ type: "draft", text: "/", selection: [1, 1] });
  await model.dispatch({ type: "focus", focused: true });
  expect(model.inspect().snapshot.slash.visible).toBe(true);
  await model.dispatch({ type: "key", key: { key: "ArrowUp" } });
  expect(model.inspect().snapshot.slash.index).toBeGreaterThan(0);
  await model.dispatch({ type: "key", key: { key: "Escape" } });
  expect(model.inspect().snapshot.slash.visible).toBe(false);
  model.dispose();
});
test("concurrent actions carry their original publication admission and cannot overwrite a later draft", async () => {
  const { model } = await nativeModel();
  const one = model.dispatch({
    type: "draft",
    text: "First",
    selection: [5, 5],
  });
  const two = model.dispatch({
    type: "draft",
    text: "Second",
    selection: [6, 6],
  });
  await one;
  await expect(two).rejects.toThrow("Stale");
  expect(model.inspect().snapshot.draft.text).toBe("First");
  model.dispose();
});
test("AccessKit tree has real group/list/button roles, exact labels and disabled Send; unsupported row counts stay explicit", async () => {
  const commits: Commit[] = [];
  const root = createRoot(async (c) => {
    commits.push(c);
    return { accepted: true, revision: c.baseRevision + 1 };
  });
  const { AgentView } = await import("./view");
  const { states } = await import("./fixtures");
  await root.render(
    <AgentView publication={states["60k"]()} send={() => {}} />,
  );
  const result = Bun.spawnSync([wire()], {
    stdin: new TextEncoder().encode(
      commits.map((commit) => JSON.stringify({ commit })).join("\n") + "\n",
    ),
  });
  expect(result.exitCode).toBe(0);
  const row = JSON.parse(result.stdout.toString().trim().split("\n").at(-1)!);
  expect(row.error).toBeUndefined();
  expect(row.diagnostics).toEqual([]);
  type AX = {
    id: number;
    role: string;
    label: string | null;
    disabled: boolean;
    rowCount: number | null;
  };
  const ax = row.accessibility as AX[];
  expect(
    ax.some((n) => n.role === "Group" && n.label === "Agent conversation"),
  ).toBe(true);
  const list = ax.find(
    (n) => n.role === "List" && n.label === "Agent conversation",
  )!;
  expect(list).toBeDefined();
  expect(list.rowCount).toBeNull();
  expect(
    ax.find((n) => n.role === "Button" && n.label === "Send")?.disabled,
  ).toBe(true);
  expect(ax.some((n) => n.label === "You said: Make the headline")).toBe(true);
  expect(ax).toHaveLength(root.inspect().nodes.length);
  await root.dispose();
});
