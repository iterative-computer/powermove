import { expect, test } from "bun:test";
import { TimelinePanel } from "../panel";
import { completeFrame, nativeTimeline } from "../native";
import type { Intent, TimelineFrame } from "../model";
import type { Json } from "../../../src/protocol";
import { recorder, nativeValidators } from "./helpers";
import { frame, layer } from "./fixtures";

function run(operations: unknown[]): any[] {
  const binary = nativeValidators().model;
  const result = Bun.spawnSync([binary], {
    stdin: Buffer.from(
      operations.map((op) => JSON.stringify(op)).join("\n") + "\n",
    ),
  });
  expect(result.exitCode).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
const window = { op: "window", first: 0, visible: 10 };
/** Synthetic schema/media presentation, not a production schema implementation. */
function presentation(native: any): TimelineFrame {
  return frame({
    window: {
      total: native.window.total,
      first: native.window.first,
      rows: native.window.rows.map((r: any) => ({
        ...layer(r.layer),
        id: r.id,
        label: r.label,
        accessibilityLabel: `${r.label}, text layer, 0:00 to 0:10`,
      })),
    },
  });
}

test("real Rust model window -> presentation authority guard -> accepted v4/AccessKit surface", async () => {
  const results = run([
    {
      op: "act",
      intent: { type: "selectRow", id: '["layer","headline"]', modifiers: {} },
    },
    {
      op: "act",
      intent: {
        type: "selectRow",
        id: '["layer","kicker"]',
        modifiers: { shiftKey: true },
      },
    },
    window,
  ]);
  expect(results.every((r) => r.modelAccepted)).toBe(true);
  const native = results.at(-1).window;
  const f = completeFrame(native, presentation(native), {
    first: 0,
    visible: 10,
  });
  expect(f.selection).toEqual({ kind: "layers", count: 2 });
  expect(f.window.rows.every((row) => row.selected)).toBe(true);
  const { root, commits } = recorder();
  await root.render(<TimelinePanel frame={f} send={() => {}} />);
  const replay = Bun.spawnSync([nativeValidators().surface], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(replay.exitCode).toBe(0);
  const accepted = replay.stdout
    .toString()
    .trim()
    .split("\n")
    .map((s) => JSON.parse(s))
    .at(-1);
  expect(accepted.accepted).toBe(true);
  expect(accepted.diagnostics).toEqual([]);
  const rows = [...root.host.nodes.values()].filter(
    (n) => n.props["data-pm-component"] === "tree-item",
  );
  expect(rows.map((row) => row.props["aria-selected"])).toEqual([true, true]);
  // The count is announced (a status label), not drawn over the rows.
  const status = [...root.host.nodes.values()].find(
    (n) => n.props.id === "timeline:selection",
  );
  expect(status?.props.label).toBe("2 layers selected");
  await root.dispose();
});

test("panel row action is executed by actual Rust selection and reflected back into row state", async () => {
  const native = run([window])[0].window;
  const f = completeFrame(native, presentation(native), {
    first: 0,
    visible: 10,
  });
  const actions: Intent[] = [];
  const { root } = recorder();
  await root.render(
    <TimelinePanel frame={f} send={(action) => actions.push(action)} />,
  );
  const row = [...root.host.nodes.values()].find(
    (n) => n.props.id === 'timeline:row:["layer","headline"]',
  )!;
  await root.drive(row.id, "click", { metaKey: true });
  const after = run([{ op: "act", intent: actions[0] }, window]).at(-1).window;
  expect(after.window.rows[0].selected).toBe(true);
  const toggled = run([
    { op: "act", intent: actions[0] },
    { op: "act", intent: actions[0] },
    window,
  ]).at(-1).window;
  expect(toggled.window.rows[0].selected).toBe(false);
  await root.dispose();
});

test("presenter cannot overwrite native identity, mode, selection, lock or exact negative timing", () => {
  const native = run([window])[0].window;
  native.window.rows[0].locked = true;
  const decorated = presentation(native);
  decorated.document = "0";
  decorated.revision = "0";
  decorated.mode = "Tracks";
  decorated.window.rows[0].locked = false;
  decorated.window.rows[0].selected = true;
  const f = completeFrame(native, decorated, { first: 0, visible: 10 });
  expect(f.document).toBe(native.document);
  expect(f.mode).toBe("Layers");
  expect(f.window.rows[0].locked).toBe(true);
  expect(f.window.rows[0].selected).toBe(false);
  expect((f.window.rows[0] as unknown as Record<string, number>).from).toBe(
    -0.125,
  );
  decorated.window.rows = [];
  expect(() =>
    completeFrame(native, decorated, { first: 0, visible: 10 }),
  ).toThrow("Missing native row presentation");
});

test("native binding waits for effect acknowledgement and forwards retirement exactly", async () => {
  const native = run([window])[0].window;
  const calls: unknown[] = [];
  const model = nativeTimeline(
    {
      async window(request) {
        calls.push(request);
        return native as Json;
      },
      async dispatch(intent) {
        calls.push(intent);
        throw new Error("transport not ready");
      },
      async dispose() {
        calls.push("disposed");
      },
    },
    {
      async present(value) {
        return presentation(value);
      },
    },
  );
  await model.inspect({ first: 0, visible: 10 });
  await expect(model.act({ type: "play" })).rejects.toThrow(
    "transport not ready",
  );
  await model.dispose();
  expect(calls).toEqual([
    { first: 0, visible: 10 },
    { type: "play" },
    "disposed",
  ]);
});

test("native fixture distinguishes model effect request from successful host work", () => {
  const result = run([{ op: "act", intent: { type: "play" } }])[0];
  expect(result).toEqual({
    modelAccepted: true,
    hostCompleted: false,
    effects: ["TogglePlayback"],
  });
});
