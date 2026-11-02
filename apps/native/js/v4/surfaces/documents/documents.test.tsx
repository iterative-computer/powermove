import { expect, test } from "bun:test";
import type { ComponentChild } from "preact";
import { createRoot, type Commit } from "../../src/index";
import { DocumentNotice, DocumentLoading, PoppedOutPanel } from "./index";
const target = { window: 7, project: "synthetic", generation: "open-1" };
async function render(element: ComponentChild) {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  await root.render(element);
  const nodes = () =>
    [...root.host.nodes.values()].filter((node) => node.id !== 0);
  return { root, commits, nodes };
}
test("notices preserve exact Rust copy and generation-qualified actions", async () => {
  for (const [action, label] of [
    ["showInFinder", "Show in Finder"],
    ["saveAs", "Save As…"],
    ["saveAgain", "Save Again"],
  ] as const) {
    const actions: unknown[] = [];
    const text = "A recovery copy of “Film” was kept.";
    const { root, nodes, commits } = await render(
      <DocumentNotice
        target={target}
        notice={{ text, level: "info", action: { id: action, label } }}
        location="viewer"
        onAction={(t, a) => actions.push([t, a])}
      />,
    );
    expect(JSON.stringify(commits)).toContain(text);
    const button = nodes().find((n) => n.props.label === label)!;
    await root.dispatch(
      {
        revision: root.inspect().revision,
        target: button.id,
        deliveries: [{ id: button.id, capture: false }],
        preventDefault: false,
      },
      "click",
      {},
    );
    expect(actions).toEqual([[target, action]]);
    const dismiss = nodes().find((n) => n.props.label === "Dismiss")!;
    expect(dismiss.props.title).toBe("Dismiss");
    expect(dismiss.props.metadata).toEqual({
      symbol: "xmark",
      borderless: true,
    });
  }
});
test("passive notices have no extra action or close button", async () => {
  const { nodes } = await render(
    <DocumentNotice
      target={target}
      notice={{ text: "“Film” was updated from another app.", level: "info" }}
      location="viewer"
      onAction={() => {
        throw new Error("no action");
      }}
    />,
  );
  expect(nodes().filter((n) => n.type === "button")).toHaveLength(0);
});
test("library notices use inline tokens and do not invent a dismiss control", async () => {
  const { nodes } = await render(
    <DocumentNotice
      target={target}
      notice={{
        text: "A recovery copy of “Film” was kept.",
        level: "info",
        action: { id: "showInFinder", label: "Show in Finder" },
      }}
      location="library"
      onAction={() => {}}
    />,
  );
  expect(nodes().some((n) => n.props.label === "Dismiss")).toBe(false);
  const group = nodes().find((n) => n.props.role === "group")!;
  expect(group.props.style).toMatchObject({
    background: { token: "colors.panel_secondary" },
    borderRadius: { token: "metrics.control_radius" },
  });
});
test("opening is absent before the Rust delay and uses indeterminate kit progress afterward", async () => {
  const { root, nodes } = await render(
    <DocumentLoading name="Film" visible={false} />,
  );
  expect(nodes()).toHaveLength(0);
  await root.render(<DocumentLoading name="Film" visible />);
  expect(
    JSON.stringify(nodes().map((n) => ({ props: n.props, text: n.data }))),
  ).toContain("Opening “Film”…");
  const progress = nodes().find((n) => n.props.role === "progressbar")!;
  expect(progress.props.label).toBe("Opening Film");
  expect(progress.props["data-pm-indeterminate"]).toBe(true);
  expect(progress.props["aria-valuenow"]).toBeUndefined();
  await root.render(<DocumentLoading name="Film" visible={false} />);
  expect(nodes()).toHaveLength(0);
});
test("popout header keeps controls and exposes the exact Return VoiceOver label", async () => {
  const { nodes } = await render(
    <PoppedOutPanel
      panelTitle="Properties"
      headerControls={<button label="Existing Control" />}
      onReturn={() => {}}
    >
      <text>Authored content</text>
    </PoppedOutPanel>,
  );
  const buttons = nodes().filter((n) => n.type === "button");
  expect(buttons.map((n) => n.props.label)).toEqual([
    "Existing Control",
    "Return Properties to Layout",
  ]);
  expect(buttons[1].props.title).toBe("Return to Layout");
  expect(buttons[1].props.metadata).toEqual({
    symbol: "rectangle.inset.filled.and.arrow.left",
    borderless: true,
  });
});
test("visual values are tokens or structural keywords", async () => {
  const { nodes } = await render(
    <PoppedOutPanel
      panelTitle="Properties"
      headerControls={null}
      onReturn={() => {}}
    >
      <DocumentLoading name="Film" visible />
    </PoppedOutPanel>,
  );
  for (const node of nodes()) {
    const styles = node.props.style as Record<string, unknown> | undefined;
    for (const value of Object.values(styles ?? {})) {
      expect(typeof value).not.toBe("number");
      if (typeof value === "object") expect(value).toHaveProperty("token");
    }
  }
});
test("document views replay through the real Rust six-op validator and accessibility projection", async () => {
  const { fileURLToPath } = await import("node:url");
  const repository = fileURLToPath(
    new URL("../../../../../../", import.meta.url),
  );
  const built = Bun.spawnSync(
    [
      "cargo",
      "build",
      "--locked",
      "-p",
      "pm-surface",
      "--example",
      "ui_kit_wire",
    ],
    { cwd: `${repository}/apps/native`, stdout: "pipe", stderr: "pipe" },
  );
  if (built.exitCode !== 0) throw new Error(built.stderr.toString());
  const { commits } = await render(
    <PoppedOutPanel
      panelTitle="Properties"
      headerControls={null}
      onReturn={() => {}}
    >
      <DocumentLoading name="Film" visible />
      <DocumentNotice
        target={target}
        notice={{
          text: "Couldn’t confirm that “Film” was saved.",
          level: "warning",
          action: { id: "saveAgain", label: "Save Again" },
        }}
        location="viewer"
        onAction={() => {}}
      />
    </PoppedOutPanel>,
  );
  const result = Bun.spawnSync(
    [`${repository}/apps/native/target/debug/examples/ui_kit_wire`],
    {
      stdin: Buffer.from(
        commits.map((c) => JSON.stringify(c)).join("\n") + "\n",
      ),
    },
  );
  expect(result.exitCode).toBe(0);
  const replies = result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  for (const reply of replies) {
    expect(reply.accepted).toBe(true);
    expect(reply.wireDiagnostics).toEqual([]);
    expect(reply.diagnostics).toEqual([]);
    expect(reply.accessibilityNodes).toBeGreaterThan(0);
    expect(reply.focusOrder.length).toBeGreaterThan(0);
  }
});
