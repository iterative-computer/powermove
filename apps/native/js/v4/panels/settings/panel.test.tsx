import { test, expect } from "bun:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { createRoot, type Commit, type SnapshotNode } from "../../src/index";
import type { SurfaceRoot } from "../../src/renderer";
import { SettingsPanel } from "./index";
import { fixture, publication, extension, bindings } from "./fixtures";
import type { SettingsModel } from "./model";
import { accessibility } from "./validation";

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
  const rows = result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((s) => JSON.parse(s));
  for (const row of rows) {
    expect(row.accepted).toBe(true);
    expect(row.wireDiagnostics).toEqual([]);
    expect(row.diagnostics).toEqual([]);
    expect(row.accessibilityNodes).toBe(Object.keys(row.nodes).length);
  }
  return rows.at(-1);
}
function opTree(row: ReturnType<typeof replay>) {
  const lines: string[] = [];
  const visit = (id: number, depth: number) => {
    const node = row.nodes[id];
    const p = node.props;
    const descriptor = [
      node.type,
      p["data-pm-component"],
      p.role,
      p.id,
      p.label && `label=${JSON.stringify(p.label)}`,
      p.value !== undefined && `value=${JSON.stringify(p.value)}`,
      p.disabled && "disabled",
      p["aria-selected"] && "selected",
      p["aria-description"] && `help=${JSON.stringify(p["aria-description"])}`,
      p.count !== undefined && `count=${p.count}`,
      p.range && `range=${p.range}`,
    ]
      .filter(Boolean)
      .join(" ");
    lines.push(
      `${"  ".repeat(depth)}${descriptor}${node.text ? ` ${JSON.stringify(node.text)}` : ""}`,
    );
    node.children.forEach((child: number) => visit(child, depth + 1));
  };
  visit(0, 0);
  const focus = row.focusOrder.map(
    (id: number) => row.nodes[id].props.id ?? row.nodes[id].props.label,
  );
  return `${lines.join("\n")}\nFocus: ${JSON.stringify(focus)}`;
}
const stateCases: [string, (model: SettingsModel) => void | Promise<void>][] = [
  ["empty", () => {}],
  [
    "loading",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        extensionsState: "loading",
        rows: [
          {
            id: "account.chatgpt",
            section: "accounts",
            group: "Subscriptions",
            label: "ChatGPT",
            control: "status",
            busy: true,
          },
        ],
      });
    },
  ],
  [
    "selected",
    (model) => {
      model.select("accounts");
    },
  ],
  // Hover and rings are system-owned. No fabricated JSX hover/focus styling.
  ["hover-system-controls", () => {}],
  ["focus-system-rings", () => {}],
  [
    "disabled",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        rows: [
          {
            id: "account.key",
            section: "accounts",
            group: "API or Local Model",
            label: "API Key",
            control: "secure",
            disabled: true,
            detail: "Optional for local models.",
          },
        ],
      });
    },
  ],
  [
    "error",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        extensionsState: "error",
        rows: [
          {
            id: "account.status",
            section: "accounts",
            group: "API or Local Model",
            label: "Status",
            control: "status",
            error: "Couldn’t connect. Check the address and model.",
          },
        ],
      });
    },
  ],
  [
    "unavailable",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        extensionsState: "unavailable",
      });
    },
  ],
  [
    "no-search-results",
    async (model) => {
      await model.search("no-fixture-match");
    },
  ],
  [
    "extension-detail",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        extensions: [extension()],
      });
      model.viewExtension("fixture-0");
    },
  ],
  ["drag-drop-not-applicable", () => {}],
  ["multi-select-not-applicable", () => {}],
  [
    "60k-extensions",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        extensions: Array.from({ length: 60_000 }, (_, i) => extension(i)),
      });
      model.viewport(30_000, 20);
    },
  ],
  [
    "project-open",
    (model) => {
      model.publish({
        ...model.publication,
        revision: 2,
        project: { id: "p", name: "Fixture" },
      });
      model.show();
    },
  ],
];
for (const [state, prepare] of stateCases)
  test(`${state}: six-op tree snapshot passes the real Rust wire and kit validators`, async () => {
    const { model } = fixture();
    model.show();
    await prepare(model);
    const { root, commits } = recorder();
    await root.render(<SettingsPanel model={model} bindings={bindings} />);
    const row = replay(commits);
    expect(opTree(row)).toMatchSnapshot();
    if (state === "60k-extensions") {
      const virtual = Object.values(row.nodes).find(
        (n: any) => n.type === "virtual-list",
      ) as any;
      expect(virtual.props.count).toBe(60_000);
      expect(virtual.children).toHaveLength(60);
      expect(Object.keys(row.nodes).length).toBeLessThan(600);
    }
    if (state.endsWith("not-applicable"))
      for (const node of root.inspect().nodes)
        expect(
          Object.keys(node.props).some((key) =>
            /ondrag|ondrop|multiselect/i.test(key),
          ),
        ).toBe(false);
    root.dispose();
    model.dispose();
  });
function find(
  root: SurfaceRoot,
  label: string,
  component?: string,
): SnapshotNode {
  const n = root
    .inspect()
    .nodes.find(
      (n) =>
        n.props.label === label &&
        (!component || n.props["data-pm-component"] === component),
    );
  if (!n) throw new Error(`Missing fixture node ${label}`);
  return n;
}
async function event(
  root: SurfaceRoot,
  target: number,
  type: string,
  detail = {},
) {
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target,
      preventDefault: true,
      deliveries: [{ id: target, capture: false }],
    },
    type,
    detail,
  );
}
test("native search input, sidebar activation and window shortcuts reach the model", async () => {
  const { model, effects } = fixture();
  model.show();
  const { root, commits } = recorder();
  const render = () =>
    root.render(<SettingsPanel model={model} bindings={bindings} />);
  await render();
  await event(root, find(root, "Search", "text-field").id, "input", {
    value: "instant",
  });
  await Promise.resolve();
  await render();
  expect(model.visibleSections).toEqual(["general"]);
  await model.search("");
  await render();
  await event(root, find(root, "Accounts", "list-item").id, "click");
  expect(model.active).toBe("accounts");
  await event(root, find(root, "Settings").id, "keydown", {
    key: "f",
    metaKey: true,
  });
  expect(effects.at(-1)).toEqual(["focus"]);
  await event(root, find(root, "Settings").id, "keydown", { key: "Escape" });
  await render();
  expect(root.inspect().nodes).toHaveLength(1);
  replay(commits);
  root.dispose();
  model.dispose();
});
test("leased native preference control writes through the model and v4 host, not a JS document", async () => {
  const f = fixture();
  f.model.show();
  f.model.publish({
    ...f.model.publication,
    revision: 2,
    rows: [
      {
        ...f.model.rows.find((row) => row.id === "appearance")!,
        binding: "test-lease.appearance",
      },
    ],
  });
  const { root, commits } = recorder();
  await root.render(<SettingsPanel model={f.model} bindings={bindings} />);
  const appearance = find(root, "Appearance", "pop-up-button");
  await event(root, appearance.id, "change", { value: "dark" });
  await Promise.resolve();
  await f.host.getSetting("themeMode");
  expect(f.settings.get("themeMode")).toBe("dark");
  expect(appearance.props.attr).toEqual({
    "aria-description":
      "Follow your system setting, or pick a light or dark theme.",
  });
  replay(commits);
  root.dispose();
  f.model.dispose();
});
test("extension row enters details, back returns without clearing search, Delete routes archive", async () => {
  const f = fixture({ extensions: [extension()] });
  f.model.show();
  f.model.viewport(0, 10);
  const { root, commits } = recorder();
  const render = () =>
    root.render(<SettingsPanel model={f.model} bindings={bindings} />);
  await render();
  await event(root, find(root, "Fixture 0", "list-item").id, "click");
  expect(f.model.detailId).toBe("fixture-0");
  await render();
  await event(root, find(root, "Delete…", "push-button").id, "click");
  await Promise.resolve();
  expect(f.actions).toEqual([{ type: "extension.archive", id: "fixture-0" }]);
  await f.model.search("Fixture");
  await render();
  await event(root, find(root, "Extensions", "toolbar-item").id, "click");
  expect(f.model.detailId).toBeUndefined();
  expect(f.model.query).toBe("Fixture");
  replay(commits);
  root.dispose();
  f.model.dispose();
});
test("AX contract has labelled groups, selected neutral sidebar, search/list first focus and no secrets", async () => {
  const f = fixture();
  f.model.show();
  const { root, commits } = recorder();
  await root.render(<SettingsPanel model={f.model} bindings={bindings} />);
  const row = replay(commits);
  const nodes = root.inspect().nodes;
  expect(nodes.find((n) => n.props.id === "settings")?.props.role).toBe(
    "group",
  );
  expect(find(root, "Settings sections", "list").props.role).toBe("list");
  expect(find(root, "General", "list-item").props["aria-selected"]).toBe(true);
  expect(find(root, "General", "list-item").props.style).toHaveProperty(
    "background",
    { token: "colors.selected_fill" },
  );
  expect(row.focusOrder.slice(0, 2)).toEqual([
    find(root, "Search", "text-field").id,
    find(root, "Settings sections", "list").id,
  ]);
  expect(
    nodes
      .filter((n) => n.props["data-pm-component"] === "text-field")
      .some((n) => n.props.label === "API Key"),
  ).toBe(false);
  expect(
    nodes.some(
      (n) => n.props.role === "group" && n.props.label === "Appearance",
    ),
  ).toBe(true);
  root.dispose();
  f.model.dispose();
});
for (const state of ["available", "downloading", "installed", "error"] as const)
  test(`transcription ${state} wire/AX state`, async () => {
    const f = fixture({
      speech: [
        {
          id: "m",
          name: "Fixture speech",
          size: "Fixture size",
          state,
          ...(state === "downloading" ? { progress: 0.25 } : {}),
          ...(state === "error" ? { error: "Fixture service error" } : {}),
        },
      ],
    });
    f.model.show();
    const { root, commits } = recorder();
    await root.render(<SettingsPanel model={f.model} bindings={bindings} />);
    expect(opTree(replay(commits))).toMatchSnapshot();
    if (state === "downloading")
      expect(find(root, "Fixture speech", "progress-row").props.value).toBe(
        0.25,
      );
    root.dispose();
    f.model.dispose();
  });
test("panel browser bundle mounts without Node, Chromium or DOM globals", async () => {
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
    settingsProbe?: Promise<unknown>;
    structuredClone: typeof structuredClone;
  } = { structuredClone };
  runInNewContext(await build.outputs[0].text(), context);
  expect(await context.settingsProbe).toEqual({
    revision: 1,
    node: "undefined",
    window: "undefined",
    document: "undefined",
    buffer: "undefined",
  });
});

test("actual AccessKit projection contains every node, group labels, selection and ordered focus", async () => {
  const f = fixture();
  f.model.show();
  const { root, commits } = recorder();
  await root.render(<SettingsPanel model={f.model} bindings={bindings} />);
  const { ax, focusOrder } = accessibility(commits);
  expect(ax).toHaveLength(root.inspect().nodes.length);
  expect(ax.find((n) => n.label === "Settings")?.role).toBe("Group");
  expect(
    ax.find((n) => n.label === "Settings sections" && n.role === "List"),
  ).toBeDefined();
  expect(
    ax.find((n) => n.label === "General" && n.role === "ListItem")?.selected,
  ).toBe(true);
  expect(
    ax.find((n) => n.label === "Appearance" && n.role === "Group"),
  ).toBeDefined();
  expect(ax.find((n) => n.label === "Search")?.role).toBe("TextInput");
  expect(focusOrder.slice(0, 2)).toEqual([
    find(root, "Search", "text-field").id,
    find(root, "Settings sections", "list").id,
  ]);
  root.dispose();
  f.model.dispose();
});

test("AX help/count requirements remain declared for the native projection", async () => {
  const f = fixture({
    extensions: [extension()],
    rows: [
      {
        id: "account.url",
        section: "accounts",
        group: "API or Local Model",
        label: "API Base URL",
        control: "text",
        value: "Fixture",
        binding: "test.url",
        detail: "Base URL of the API, ending in /v1.",
      },
    ],
  });
  f.model.show();
  f.model.viewport(0, 1);
  const { root, commits } = recorder();
  await root.render(<SettingsPanel model={f.model} bindings={bindings} />);
  const { ax } = accessibility(commits);
  // Permit the owning surface lane to fix projection without pinning a gap as
  // desired behavior. Until then the semantic requirements remain on the wire.
  const field = find(root, "API Base URL", "text-field");
  expect(field.props.attr).toEqual({
    "aria-description": "Base URL of the API, ending in /v1.",
  });
  const input = ax.find((n) => n.id === field.id)!;
  expect(input).toBeDefined();
  if (input.help !== null)
    expect(input.help).toBe("Base URL of the API, ending in /v1.");
  const list = root.inspect().nodes.find((n) => n.type === "virtual-list")!;
  expect(list.props.metadata).toHaveProperty("rowCount", 1);
  const projectedList = ax.find((n) => n.id === list.id)!;
  expect(projectedList).toBeDefined();
  if (projectedList.role === "List") expect(projectedList.rowCount).toBe(1);
  root.dispose();
  f.model.dispose();
});

test("view visual values are pm-design tokens, never lane-authored numbers or colors", async () => {
  const source = await Bun.file(new URL("./index.tsx", import.meta.url)).text();
  const ast = ts.createSourceFile(
    "settings.tsx",
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const problems: string[] = [];
  const visual =
    /^(color|background|.*Color|fontSize|fontFamily|fontWeight|lineHeight|symbolSize|height|width|minWidth|maxWidth|minHeight|maxHeight|padding.*|margin.*|gap|.*Radius|.*Width|duration)$/;
  const visit = (node: ts.Node) => {
    if (ts.isPropertyAssignment(node) && visual.test(node.name.getText(ast))) {
      const v = node.initializer;
      if (!ts.isCallExpression(v) || v.expression.getText(ast) !== "t")
        problems.push(node.getText(ast));
    }
    ts.forEachChild(node, visit);
  };
  visit(ast);
  expect(problems).toEqual([]);
  expect(source).not.toMatch(
    /<webview|dangerouslySetInnerHTML|<UI.Checkbox|<UI.Toggle/,
  );
});

test("frozen-contract native key delivery never requests retroactive cancellation", async () => {
  const f = fixture();
  f.model.show();
  const { root, commits } = recorder();
  await root.render(<SettingsPanel model={f.model} bindings={bindings} />);
  // Plan cancellation from actual accepted listener flags instead of marking
  // every synthetic event already canceled. Modifier-aware native planning is
  // a shell/API request, not a synchronous JS decision.
  const target = find(root, "Search", "text-field").id;
  await root.drive(target, "keydown", { key: "f", metaKey: true });
  expect(f.effects.at(-1)).toEqual(["focus"]);
  expect(root.inspect().fenced).toBe(false);
  await root.drive(target, "keydown", { key: "f" });
  expect(f.model.open).toBe(true);
  replay(commits);
  root.dispose();
  f.model.dispose();
});
