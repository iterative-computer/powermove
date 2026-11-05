import { test, expect } from "bun:test";
import { h } from "preact";
import { createRoot } from "../../src";
import { SourceError, TaskBody, type Target, type TaskModel } from ".";
const target: Target = {
  owner: { window: "w", project: "p", document: "1", revision: "4" },
  id: "source",
  generation: "1",
};
function root() {
  return createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
}
test("source error renders approved kit, copy, symbol and VoiceOver action", async () => {
  const r = root();
  const calls: unknown[] = [];
  await r.render(
    h(SourceError, {
      model: {
        target,
        title: "Can’t Preview “Clip.mov”",
        line: "The file is missing.",
        button: "locate",
      },
      warningSymbol: "opaque-warning-symbol",
      warningSymbolSize: 26,
      dispatch: (...args) => calls.push(args),
    }),
  );
  expect(r.host.root.textContent).toBe(
    "Can’t Preview “Clip.mov”The file is missing.Locate File…",
  );
  const nodes = [...r.host.nodes.values()];
  expect(
    nodes.some((n) => n.props["data-pm-component"] === "empty-state"),
  ).toBe(true);
  expect(nodes.find((n) => n.type === "img")?.props.resource).toBe(
    "opaque-warning-symbol",
  );
  const button = nodes.find((n) => n.props.label === "Locate File…")!;
  await r.dispatch(
    {
      revision: r.inspect().revision,
      target: button.id,
      preventDefault: false,
      deliveries: [{ id: button.id, capture: false }],
    },
    "click",
    {},
  );
  expect(calls).toEqual([[target, "locate"]]);
});
test("playback and audio warnings preserve video by rendering no empty state", async () => {
  const r = root();
  await r.render(
    h(SourceError, {
      model: { target, title: null, line: null, button: null },
      warningSymbol: "warning",
      warningSymbolSize: 26,
      dispatch: () => {},
    }),
  );
  expect(r.host.root.textContent).toBe("");
  expect(
    [...r.host.nodes.values()].some(
      (n) => n.props["data-pm-component"] === "empty-state",
    ),
  ).toBe(false);
});
const task: TaskModel = {
  target,
  title: "Importing 2 files",
  detail: "1 of 2",
  progress: 0.5,
  severity: "warning",
  cancel: true,
  retry: false,
  disclosure: [{ file: "a.mov", reason: "The file is missing" }],
};
test("task body carries measured progress and disclosure is absent until expanded", async () => {
  const r = root();
  await r.render(
    h(TaskBody, { model: task, expanded: false, dispatch: () => {} }),
  );
  expect(r.host.root.textContent).toBe("Importing 2 files1 of 2Cancel");
  const p = [...r.host.nodes.values()].find(
    (n) => n.props.role === "progressbar",
  )!;
  expect(p.props["aria-valuenow"]).toBe(0.5);
  await r.render(
    h(TaskBody, {
      model: { ...task, progress: undefined },
      expanded: true,
      dispatch: () => {},
    }),
  );
  expect(r.host.root.textContent).toContain("a.movThe file is missing");
  expect(
    [...r.host.nodes.values()].find((n) => n.props.role === "progressbar")!
      .props["data-pm-indeterminate"],
  ).toBe(true);
});
test("focused running row Escape queues cancellation by stable owner and id", async () => {
  const r = root();
  const calls: unknown[] = [];
  await r.render(
    h(TaskBody, {
      model: task,
      expanded: false,
      dispatch: (...args) => calls.push(args),
    }),
  );
  const row = [...r.host.nodes.values()].find((n) => n.props.role === "group")!;
  await r.dispatch(
    {
      revision: r.inspect().revision,
      target: row.id,
      preventDefault: true,
      deliveries: [{ id: row.id, capture: false }],
    },
    "keydown",
    { key: "Escape" },
  );
  expect(calls).toEqual([[target, "cancel"]]);
});
test("terminal row has Retry and cannot cancel via Escape", async () => {
  const r = root();
  const calls: unknown[] = [];
  await r.render(
    h(TaskBody, {
      model: { ...task, cancel: false, retry: true },
      expanded: false,
      dispatch: (...args) => calls.push(args),
    }),
  );
  const nodes = [...r.host.nodes.values()];
  expect(nodes.some((n) => n.props.label === "Cancel")).toBe(false);
  const row = nodes.find((n) => n.props.role === "group")!;
  await r.dispatch(
    {
      revision: r.inspect().revision,
      target: row.id,
      preventDefault: false,
      deliveries: [{ id: row.id, capture: false }],
    },
    "keydown",
    { key: "Escape" },
  );
  expect(calls).toEqual([]);
  const retry = nodes.find((n) => n.props.label === "Retry")!;
  await r.dispatch(
    {
      revision: r.inspect().revision,
      target: retry.id,
      preventDefault: false,
      deliveries: [{ id: retry.id, capture: false }],
    },
    "click",
    {},
  );
  expect(calls).toEqual([[target, "retry"]]);
});
