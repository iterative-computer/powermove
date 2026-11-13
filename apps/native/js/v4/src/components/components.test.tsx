import { fileURLToPath } from "node:url";
import { test, expect } from "bun:test";
import type { ComponentChild } from "preact";
import { createRoot, type Commit } from "../index";
import type { SurfaceRoot } from "../renderer";
import * as UI from "./index";
import ts from "typescript";

function recorder() {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits };
}
function replay(commits: Commit[]) {
  const wire = process.env.PM_UI_KIT_WIRE;
  if (!wire)
    throw new Error(
      "Build ui_kit_wire and set PM_UI_KIT_WIRE to run component conformance",
    );
  const result = Bun.spawnSync([wire], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(result.exitCode).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
function node(root: SurfaceRoot, kind: string) {
  return [...root.host.nodes.values()].find(
    (n) => n.props["data-pm-component"] === kind,
  )!;
}
async function event(
  root: SurfaceRoot,
  id: number,
  type: string,
  key?: string,
) {
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: id,
      preventDefault: true,
      deliveries: [{ id, capture: false }],
    },
    type,
    { key },
  );
}
const cases: [string, () => ComponentChild][] = [
  [
    "inspector-row",
    () => (
      <UI.InspectorRow label="Position">
        <UI.NumberField value={12} binding="x" unit="px" />
      </UI.InspectorRow>
    ),
  ],
  [
    "number-field",
    () => (
      <UI.NumberField label="Opacity" value={100} binding="opacity" unit="%" />
    ),
  ],
  [
    "text-field",
    () => <UI.TextField label="Name" value="Authored 🎬" binding="name" />,
  ],
  [
    "text-field-multiline",
    () => (
      <UI.TextField
        label="Text"
        value={"Made with\nPowermove"}
        binding="text"
        multiline
      />
    ),
  ],
  [
    "pop-up-button",
    () => (
      <UI.PopUpButton
        label="Quality"
        value={null}
        binding="quality"
        options={[
          { v: null, label: "Auto" },
          { v: false, label: "Draft" },
        ]}
      />
    ),
  ],
  [
    "segmented-control",
    () => (
      <UI.SegmentedControl
        label="Axis"
        value="x"
        options={[{ id: "x", label: "X" }]}
        onChange={() => {}}
      />
    ),
  ],
  [
    "checkbox",
    () => <UI.Checkbox label="Visible" value={false} binding="visible" mixed />,
  ],
  [
    "toggle",
    () => <UI.Toggle label="Enabled" value={true} binding="enabled" />,
  ],
  [
    "colour-well",
    () => <UI.ColourWell label="Colour" value="#123456" binding="colour" />,
  ],
  [
    "disclosure-group",
    () => (
      <UI.DisclosureGroup label="Transform" open onChange={() => {}}>
        <UI.TextField label="Name" value="Layer" binding="name" />
      </UI.DisclosureGroup>
    ),
  ],
  [
    "list",
    () => (
      <UI.List label="Effects">
        <UI.ListItem label="Blur" selected onSelect={() => {}} />
      </UI.List>
    ),
  ],
  [
    "tree",
    () => (
      <UI.Tree label="Layers">
        <UI.TreeItem
          label="Group"
          expanded
          onExpand={() => {}}
          onSelect={() => {}}
        >
          <UI.Tree label="Children">
            <UI.TreeItem label="Layer" onSelect={() => {}} />
          </UI.Tree>
        </UI.TreeItem>
      </UI.Tree>
    ),
  ],
  ["toolbar-item", () => <UI.ToolbarItem label="Select" active />],
  [
    "sheet-anchor",
    () => (
      <UI.SheetAnchor label="Composition Settings" anchor={0} open>
        <UI.TextField label="Name" binding="name" value="Composition" />
      </UI.SheetAnchor>
    ),
  ],
  [
    "popover-anchor",
    () => (
      <UI.PopoverAnchor label="Colour" anchor={0} open onClose={() => {}}>
        <UI.ColourWell label="Colour" binding="colour" value="#abcdef" />
      </UI.PopoverAnchor>
    ),
  ],
  ["progress-row", () => <UI.ProgressRow label="Rendering" value={0.25} />],
  [
    "empty-state",
    () => (
      <UI.EmptyState
        label="No Effects"
        explanation="Add an effect to this layer."
        action={<UI.ToolbarItem label="Add Effect" />}
      />
    ),
  ],
];
for (const [kind, fixture] of cases) {
  test(`${kind}: JSX commits pass the real Rust panel validator`, async () => {
    const { root, commits } = recorder();
    await root.render(fixture());
    // A variant case (`-multiline`) is still its component's node.
    const n = node(root, kind.replace(/-multiline$/, ""));
    expect(n).toBeDefined();
    if (kind.endsWith("-multiline")) expect(n.type).toBe("textarea");
    const rows = replay(commits);
    for (const row of rows) {
      expect(row.accepted).toBe(true);
      expect(row.diagnostics).toEqual([]);
      expect(row.wireDiagnostics).toEqual([]);
      expect(row.accessibilityNodes).toBe(Object.keys(row.nodes).length);
    }
    expect(rows.at(-1).components).toContain(n.id);
    root.dispose();
  });
}
test("row association, shared token column, unit colour and authored numeric descriptors survive the wire", async () => {
  const { root, commits } = recorder();
  await root.render(
    <UI.InspectorRow label="Position X">
      <UI.NumberField
        value={42.125}
        binding="original"
        min={-100}
        max={200}
        step={0.25}
        precision={3}
        unit="px"
      />
    </UI.InspectorRow>,
  );
  const n = node(root, "number-field");
  const row = replay(commits).at(-1);
  expect(row.nodes[n.id].props).toMatchObject({
    value: 42.125,
    binding: "original",
    step: 0.25,
    precision: 3,
    unit: "px",
    min: -100,
    max: 200,
  });
  expect(n.props.style).toMatchObject({
    unitColor: UI.token("colors.muted"),
    fontVariantNumeric: "tabular-nums",
    textAlign: "left",
  });
  // The host sizes a row's columns: the value slot pins no width.
  expect(
    [...root.host.nodes.values()].find(
      (n) => n.props["data-pm-slot"] === "value",
    )!.props.style,
  ).toBeUndefined();
  expect(n.props.labelledBy).toBeDefined();
  root.dispose();
});
test("segmented keyboard navigation skips disabled choices, requests focus and keeps one tab stop", async () => {
  const { root, commits } = recorder();
  const changes: string[] = [];
  const options = [
    { id: "x", label: "X" },
    { id: "y", label: "Y", disabled: true },
    { id: "z", label: "Z" },
  ];
  const render = (value: string) => (
    <UI.SegmentedControl
      label="Axis"
      options={options}
      value={value}
      onChange={(id) => changes.push(id)}
    />
  );
  await root.render(render("x"));
  const segments = [...root.host.nodes.values()].filter(
    (n) => n.props["data-pm-component"] === "segment",
  );
  await event(root, segments[0].id, "keydown", "ArrowRight");
  expect(changes).toEqual(["z"]);
  expect(node(root, "segmented-control").props.focusRequest).toBe(
    segments[2].props.id,
  );
  await root.render(render("z"));
  const row = replay(commits).at(-1);
  expect(row.diagnostics).toEqual([]);
  expect(row.focusOrder).toEqual([segments[2].id]);
  await event(root, segments[2].id, "keydown", "Home");
  expect(changes).toEqual(["z", "x"]);
  root.dispose();
});
test("disclosure removes hidden descendants from focus and accessibility together", async () => {
  const { root, commits } = recorder();
  let open = true;
  const render = () => (
    <UI.DisclosureGroup
      label="Transform"
      open={open}
      onChange={(v) => (open = v)}
    >
      <UI.TextField label="Name" value="Layer" binding="name" />
    </UI.DisclosureGroup>
  );
  await root.render(render());
  const hiddenId = node(root, "text-field").id;
  await event(root, node(root, "disclosure-button").id, "click");
  await root.render(render());
  const row = replay(commits).at(-1);
  expect(row.nodes[hiddenId]).toBeUndefined();
  expect(row.focusOrder).toEqual([node(root, "disclosure-button").id]);
  root.dispose();
});
test("list and tree expose selected state, neutral fill, full keyboard actions and disabled guards", async () => {
  const { root, commits } = recorder();
  const actions: unknown[] = [];
  await root.render(
    <UI.Tree label="Layers">
      <UI.TreeItem
        label="Group"
        selected
        expanded
        onSelect={() => actions.push("select")}
        onExpand={(v) => actions.push(v)}
      />
      <UI.TreeItem
        label="Locked"
        disabled
        onSelect={() => actions.push("wrong")}
      />
    </UI.Tree>,
  );
  const item = node(root, "tree-item");
  expect(item.props.style).toMatchObject({
    background: UI.token("colors.selected_fill"),
  });
  await event(root, item.id, "keydown", "ArrowLeft");
  await event(root, item.id, "keydown", "Enter");
  const disabled = [...root.host.nodes.values()].find(
    (n) => n.props.label === "Locked",
  )!;
  await event(root, disabled.id, "click");
  expect(actions).toEqual([false, "select"]);
  expect(replay(commits).at(-1).focusOrder).toEqual([item.id]);
  root.dispose();
});
test("popover Esc closes the innermost transient; closed anchors retain no subtree", async () => {
  const { root, commits } = recorder();
  let open = true;
  const render = () => (
    <UI.PopoverAnchor
      label="Colour"
      anchor={0}
      open={open}
      onClose={() => (open = false)}
    >
      <UI.TextField label="Hex" value="abcdef" binding="colour" />
    </UI.PopoverAnchor>
  );
  await root.render(render());
  await event(root, node(root, "popover-anchor").id, "keydown", "Escape");
  await root.render(render());
  expect(replay(commits).at(-1).accessibilityNodes).toBe(1);
  expect(root.host.root.children).toHaveLength(0);
  root.dispose();
});
test("progress has explicit determinate and indeterminate states without a fake percentage", async () => {
  const { root } = recorder();
  await root.render(<UI.ProgressRow label="Rendering" />);
  expect(node(root, "progress-row").props["data-pm-indeterminate"]).toBe(true);
  expect(node(root, "progress-row").props.value).toBeUndefined();
  for (const value of [NaN, Infinity, -0.1, 1.1])
    expect(() => UI.ProgressRow({ label: "Rendering", value })).toThrow();
  root.dispose();
});
test("wire validator flags missing names, visual literals and undersized hit targets", async () => {
  const { root, commits } = recorder();
  await root.render(
    <UI.TextField
      value="text"
      binding="name"
      style={{
        height: 22,
        background: "red",
        minHeight: UI.token("metrics.field"),
      }}
    />,
  );
  const row = replay(commits).at(-1);
  expect(row.accepted).toBe(true);
  expect(row.diagnostics.map((d: { code: string }) => d.code).sort()).toEqual(
    ["missingLabel", "nonTokenValue", "nonTokenValue", "smallHitTarget"].sort(),
  );
  root.dispose();
});

function literalViolations(source: string) {
  const ast = ts.createSourceFile(
    "components.ts",
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  const problems: string[] = [];
  const visual =
    /^(color|background|.*Color|fontSize|fontFamily|fontWeight|lineHeight|symbolSize|height|width|minWidth|maxWidth|minHeight|maxHeight|padding.*|margin.*|gap|.*Radius|.*Width|duration|response|bounce|startScale)$/;
  const visit = (node: ts.Node) => {
    if (ts.isPropertyAssignment(node) && visual.test(node.name.getText(ast))) {
      const v = node.initializer;
      if (!ts.isCallExpression(v) || v.expression.getText(ast) !== "token")
        problems.push(node.getText(ast));
    }
    ts.forEachChild(node, visit);
  };
  visit(ast);
  return problems;
}
test("component styles reject literal colours, sizes, radii, fonts and durations", async () => {
  const source = await Bun.file(new URL("./index.ts", import.meta.url)).text();
  expect(literalViolations(source)).toEqual([]);
  expect(
    literalViolations(
      'const style = { color: "#123456", height: 22, borderRadius: 6, fontSize: 12, duration: 0.32 }',
    ),
  ).toHaveLength(5);
});

test("UI kit bundles and mounts with no Node, Chromium, window, Buffer or global document", async () => {
  const { runInNewContext } = await import("node:vm");
  const build = await Bun.build({
    entrypoints: [
      fileURLToPath(new URL("./nodefree-fixture.tsx", import.meta.url)),
    ],
    target: "browser",
    format: "iife",
    minify: true,
  });
  expect(build.success).toBe(true);
  const context: {
    kitProbe?: Promise<unknown>;
    structuredClone: typeof structuredClone;
  } = { structuredClone };
  runInNewContext(await build.outputs[0].text(), context);
  expect(await context.kitProbe).toEqual({
    revision: 1,
    components: 3,
    process: "undefined",
    document: "undefined",
    window: "undefined",
    buffer: "undefined",
  });
});

test("inspector leading and trailing actions stay outside the shared value column", async () => {
  const { root, commits } = recorder();
  await root.render(
    <UI.InspectorRow
      label="Scale"
      leading={<UI.ToolbarItem label="Toggle Keyframe" />}
      trailing={<UI.ToolbarItem label="Link Axes" />}
    >
      <UI.NumberField value={100} binding="scale" unit="%" />
    </UI.InspectorRow>,
  );
  const slots = node(root, "inspector-row").children.map(
    (n) => n.props["data-pm-slot"],
  );
  expect(slots).toEqual(["leading", "label", "value", "trailing"]);
  expect(replay(commits).at(-1).diagnostics).toEqual([]);
  root.dispose();
});
