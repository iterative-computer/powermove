import { test, expect } from "bun:test";
import type { ComponentChild } from "preact";
import { createRoot, type Commit } from "../index";
import * as UI from "./index";

function recorder() {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits };
}
function replay(commits: Commit[]) {
  const result = Bun.spawnSync([process.env.PM_UI_KIT_WIRE!], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(result.exitCode).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((s) => JSON.parse(s));
}
const fixtures: [string, () => ComponentChild][] = [
  [
    "switch",
    () => (
      <UI.SettingsRoot label="Settings">
        <UI.Switch label="Reopen Last Project" value binding="reopen" />
      </UI.SettingsRoot>
    ),
  ],
  [
    "secure-field",
    () => <UI.SecureField label="API Key" binding="keychain:provider" />,
  ],
  [
    "colour-well",
    () => (
      <UI.ColorWell
        label="Background"
        value="color(display-p3 1 0.125 0 / .7)"
        binding="background"
        minimal
      />
    ),
  ],
  [
    "colour-popover",
    () => (
      <UI.ColorPopover
        label="Background"
        binding="background"
        anchor={0}
        open
        fill
        onClose={() => {}}
      />
    ),
  ],
  [
    "form-group",
    () => (
      <UI.FormGroup title="Appearance">
        <UI.FormRow
          label="Appearance"
          detail="Follow your system setting."
          control={
            <UI.PopUpButton
              label="Appearance"
              value="system"
              binding="scheme"
              options={["system"]}
            />
          }
        />
      </UI.FormGroup>
    ),
  ],
  [
    "viewer-gizmo",
    () => (
      <UI.ViewerGizmo
        label="Scale Top Left"
        kind="transform"
        position={{ x: 12.5, y: -2 }}
      />
    ),
  ],
  [
    "viewer-overlay",
    () => (
      <UI.ViewerOverlay label="Viewer Controls">
        <UI.PopUpButton
          id="viewer.zoom"
          label="Zoom"
          value="fit"
          binding="zoom"
          widthSample="Fit (100 %)"
          options={[
            { v: "fit", label: "Fit" },
            { v: "separator", label: "", separator: true },
            { v: "in", label: "Zoom In", keyEquivalent: "⌘=" },
          ]}
        />
      </UI.ViewerOverlay>
    ),
  ],
  [
    "banner",
    () => (
      <UI.Banner
        label="Media offline in this frame"
        severity="warning"
        symbol="exclamationmark.triangle"
        action={<UI.PushButton label="Retry" />}
        onDismiss={() => {}}
      />
    ),
  ],
  [
    "empty-state",
    () => (
      <UI.EmptyState
        label="No Composition"
        explanation="Open a project to start editing."
        symbol="sidebar.trailing"
      />
    ),
  ],
  ["push-button", () => <UI.PushButton label="Delete…" role="destructive" />],
];
for (const [kind, fixture] of fixtures) {
  test(`${kind}: additive wire snapshot and real Rust publication`, async () => {
    const { root, commits } = recorder();
    await root.render(fixture());
    const rows = replay(commits);
    for (const row of rows) {
      expect(row.accepted).toBe(true);
      expect(row.wireDiagnostics).toEqual([]);
      expect(
        row.diagnostics.filter(
          (d: { code: string }) => d.code !== "missingDesignToken",
        ),
      ).toEqual([]);
      expect(row.accessibilityNodes).toBe(Object.keys(row.nodes).length);
    }
    const row = rows.at(-1);
    const n = Object.values(row.nodes).find(
      (n: any) => n.props["data-pm-component"] === kind,
    ) as any;
    expect(row.components).toContain(n.id);
    expect(commits).toMatchSnapshot();
    root.dispose();
  });
}

test("Switch outside Settings is diagnosed without changing the old Toggle checkbox", async () => {
  const { root, commits } = recorder();
  await root.render(<UI.Switch label="Invalid switch" binding="test" value />);
  expect(
    replay(commits)
      .at(-1)
      .diagnostics.map((d: any) => d.code),
  ).toEqual(["switchOutsideSettings"]);
  await root.render(<UI.Toggle label="Enabled" binding="test" value />);
  expect(replay(commits).at(-1).diagnostics).toEqual([]);
  root.dispose();
});
test("SecureField rejects secret values and callbacks before any wire publication", () => {
  for (const extra of [
    { value: "synthetic-sensitive-text" },
    { control: "text" },
    { onInput: () => {} },
    { onChange: () => {} },
    { children: "secret" },
  ]) {
    expect(() =>
      UI.SecureField({
        label: "API Key",
        binding: "keychain:provider",
        ...extra,
      } as any),
    ).toThrow("only a native binding");
  }
});
test("ColourPopover Esc cancels and closed content is removed", async () => {
  const { root, commits } = recorder();
  const closes: boolean[] = [];
  const render = (open: boolean) => (
    <UI.ColourPopover
      label="Background"
      binding="background"
      anchor={0}
      open={open}
      onClose={(cancel) => closes.push(cancel)}
    />
  );
  await root.render(render(true));
  const n = [...root.host.nodes.values()].find(
    (n) => n.props["data-pm-component"] === "colour-popover",
  )!;
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: n.id,
      preventDefault: true,
      deliveries: [{ id: n.id, capture: false }],
    },
    "keydown",
    { key: "Escape" },
  );
  expect(closes).toEqual([true]);
  await root.render(render(false));
  expect(replay(commits).at(-1).accessibilityNodes).toBe(1);
  root.dispose();
});
test("menu additions preserve the frozen primitive schema and typed values", async () => {
  const { root, commits } = recorder();
  await root.render(
    <UI.PopUpButton
      label="Resolution"
      binding="quality"
      value={null}
      widthSample="Auto (Quarter)"
      options={[
        {
          v: null,
          label: "Auto",
          detail: "Lowers resolution while playing to keep up",
        },
        { v: "separator", label: "", separator: true },
        { v: false, label: "Full", keyEquivalent: "⌘1" },
      ]}
    />,
  );
  const row = replay(commits).at(-1);
  expect(row.wireDiagnostics).toEqual([]);
  const n = Object.values(row.nodes).find(
    (n: any) => n.props["data-pm-component"] === "pop-up-button",
  ) as any;
  expect(n.props.options).toEqual([
    { v: null, label: "Auto" },
    { v: false, label: "Full" },
  ]);
  expect(n.props.metadata.pmKit.widthSample).toBe("Auto (Quarter)");
  expect(n.props.metadata.pmKit.menu[1].separator).toBe(true);
  expect(n.props.metadata.pmKit.menu[2].keyEquivalent).toBe("⌘1");
  root.dispose();
});
test("gizmo authored geometry is exact, finite and never scaled in JSX", async () => {
  for (const kind of [
    "transform",
    "rotation",
    "anchor",
    "path-vertex",
    "tangent",
  ] as const) {
    const { root, commits } = recorder();
    await root.render(
      <UI.ViewerGizmo
        label={kind}
        kind={kind}
        position={{ x: -0.125, y: 1000000.25 }}
        selected
      />,
    );
    expect(
      replay(commits)
        .at(-1)
        .diagnostics.filter((d: any) => d.code !== "missingDesignToken"),
    ).toEqual([]);
    root.dispose();
  }
  expect(() =>
    UI.ViewerGizmo({
      label: "Handle",
      kind: "transform",
      position: { x: NaN, y: 0 },
    }),
  ).toThrow("finite");
});
test("banner action and dismissal callbacks dispatch by stable id", async () => {
  const { root, commits } = recorder();
  let dismissed = 0;
  await root.render(
    <UI.Banner
      label="Couldn’t draw this frame. Showing the last good frame."
      severity="error"
      symbol="exclamationmark.triangle"
      action={<UI.PushButton label="Retry" />}
      onDismiss={() => dismissed++}
    />,
  );
  const n = [...root.host.nodes.values()].find(
    (n) => n.props.label === "Dismiss",
  )!;
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: n.id,
      preventDefault: true,
      deliveries: [{ id: n.id, capture: false }],
    },
    "click",
    {},
  );
  expect(dismissed).toBe(1);
  expect(replay(commits).at(-1).diagnostics).toEqual([]);
  root.dispose();
});
