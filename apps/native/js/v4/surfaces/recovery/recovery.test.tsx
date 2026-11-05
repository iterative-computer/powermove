import { expect, test } from "bun:test";
import { h, createRoot, type Commit } from "../../src";
import { RecoveryContentView, type RecoveryContent } from "./index";

function recorder() {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits };
}
const failure: RecoveryContent = {
  kind: "empty",
  title: "Tools Stopped Responding",
  line: "Powermove stopped it and will restart it in a moment.",
  button: "Restart Now",
  axLabel:
    "Tools Stopped Responding. Powermove stopped it and will restart it in a moment.",
  announcement: "high",
};
test("EmptyState uses exact AX group copy and one generation-fenced action", async () => {
  const { root, commits } = recorder();
  await root.render(
    <RecoveryContentView
      content={failure}
      view="synthetic-view"
      generation={7}
      restart={() => {}}
    />,
  );
  const group = root.host.root.children[0];
  expect(group.props["data-pm-component"]).toBe("empty-state");
  expect(group.props.label).toBe(failure.axLabel);
  expect(root.host.root.textContent).toBe(
    failure.title + failure.line + failure.button,
  );
  const buttons = commits.flatMap((c) =>
    c.ops.filter((o) => o.op === "create" && o.type === "button"),
  );
  expect(buttons).toHaveLength(1);
  await root.dispose();
});
test("realm alert leaves no panel restart button", async () => {
  const { root, commits } = recorder();
  await root.render(
    <RecoveryContentView
      content={{ ...failure, button: null }}
      view="synthetic-view"
      generation={7}
      restart={() => {
        throw new Error("button must not exist");
      }}
    />,
  );
  expect(
    commits.flatMap((c) =>
      c.ops.filter((o) => o.op === "create" && o.type === "button"),
    ),
  ).toHaveLength(0);
  await root.dispose();
});
test("ready and sub-delay starting preserve mounted content", async () => {
  const { root } = recorder();
  await root.render(
    <RecoveryContentView
      content={{ kind: "unchanged" }}
      view="synthetic-view"
      generation={7}
      restart={() => {}}
    >
      <text>Existing authored panel</text>
    </RecoveryContentView>,
  );
  expect(root.host.root.textContent).toBe("Existing authored panel");
  await root.dispose();
});
test("loading has a system indeterminate spinner and exact label", async () => {
  const { root } = recorder();
  await root.render(
    <RecoveryContentView
      content={{ kind: "loading", line: "Starting Example…" }}
      view="synthetic-view"
      generation={7}
      restart={() => {}}
    />,
  );
  const group = root.host.root.children[0];
  expect(group.props.label).toBe("Starting Example…");
  expect(group.children[0].props["data-pm-indeterminate"]).toBe(true);
  expect(root.host.root.textContent).toBe("Starting Example…");
  await root.dispose();
});
test("all visual values are token references or structural layout keywords", async () => {
  const { root, commits } = recorder();
  await root.render(
    <RecoveryContentView
      content={failure}
      view="synthetic-view"
      generation={7}
      restart={() => {}}
    />,
  );
  for (const op of commits.flatMap((c) => c.ops)) {
    if (op.op !== "setProps" || !op.patch.style) continue;
    for (const [key, value] of Object.entries(op.patch.style)) {
      if (
        ["display", "flexDirection", "alignItems", "justifyContent"].includes(
          key,
        )
      )
        continue;
      expect(typeof value).toBe("object");
      expect(value).toHaveProperty("token");
    }
  }
  await root.dispose();
});

test("native click returns the exact view generation and stale deliveries fail", async () => {
  const { root } = recorder();
  const requests: { view: string; generation: number }[] = [];
  await root.render(
    <RecoveryContentView
      content={failure}
      view="view-id"
      generation={9}
      restart={(r) => requests.push(r)}
    />,
  );
  const button = [...root.host.nodes.values()].find(
    (node) => node.type === "button",
  )!;
  const plan = {
    revision: 1,
    target: button.id,
    preventDefault: false,
    deliveries: [{ id: button.id, capture: false }],
  };
  await root.dispatch(plan, "click");
  expect(requests).toEqual([{ view: "view-id", generation: 9 }]);
  await expect(
    root.dispatch({ ...plan, revision: 0 }, "click"),
  ).rejects.toThrow("Stale");
  expect(requests).toHaveLength(1);
  await root.dispose();
});

test("review follow-up mounts the kit symbol slot without requiredSymbol placeholders", async () => {
  const { root, commits } = recorder();
  await root.render(
    <RecoveryContentView
      content={failure}
      view="view"
      generation={4}
      restart={() => {}}
    />,
  );
  const nodes = [...root.host.nodes.values()];
  const slot = nodes.find((n) => n.props["data-pm-slot"] === "symbol")!;
  expect(slot.props.metadata).toEqual({
    pmKit: { symbol: "exclamationmark.triangle", decorative: true },
  });
  const group = nodes.find(
    (n) => n.props["data-pm-component"] === "empty-state",
  )!;
  expect(group.props.metadata).toEqual({ pmKit: { severity: "warning" } });
  expect(JSON.stringify(commits)).not.toContain("requiredSymbol");
});
