import { expect, test } from "bun:test";
import { ProjectView, type ProjectViewProps } from "../index";
import { asset, comp, folder, projection } from "./fixtures";
import {
  card,
  event,
  find,
  layout,
  native,
  recorder,
  replay,
  treeSnapshot,
} from "./helpers";
import { accessibleName, meta, status } from "../presentation";
import type { Input, Projection } from "../contract";

const selected = asset();
selected.selected = true;
const focused = asset();
focused.focused = true;
const multi = [asset("a"), asset("b")].map((r) => ({ ...r, selected: true }));
const inFolder = projection([]);
inFolder.folder = "f";
inFolder.crumbs = [{ id: "f", name: "Shots", parent: null, items: 0 }];
const dropping = projection([folder(), asset()]);
dropping.external_drag = true;
const folderDrop = projection([folder()]);
folderDrop.drop_target = { folder: "f" };
const rename = projection([folder()]);
rename.renaming = "f";
const drag = projection([selected]);
drag.drag = {
  primary: { Asset: "a" },
  assets: ["a"],
  name: "a.mov",
  kind: "video",
  duration: 12,
  copy_to_timeline: true,
  move_to_bin: true,
};
const huge = projection(Array.from({ length: 12 }, (_, i) => asset(`a${i}`)));
huge.page.total = 60_000;
huge.page.next = 12;
const cases: [string, Projection | null, Partial<ProjectViewProps>?][] = [
  ["no-project", null],
  ["empty-project", projection()],
  ["empty-folder", inFolder],
  ["restoring-before-delay", projection(), { restoring: true }],
  [
    "restoring-after-delay",
    projection(),
    { restoring: true, restoringEmptyAfterDelay: true },
  ],
  ["loading-cards", projection([asset("a", "Loading")])],
  ["importing-cards", projection([asset(), asset("b", "Loading")])],
  ["selected", projection([selected])],
  ["focus", projection([focused])],
  ["multi-select", projection(multi)],
  ["disabled-composition", projection([{ ...comp(), focused: true }])],
  ["offline", projection([asset("a", "Offline")])],
  [
    "unavailable",
    projection([asset("a", { Failed: "Synthetic decode failure" })]),
  ],
  ["unknown-original", projection([asset("a", "Unknown")])],
  [
    "cloud-stored",
    projection([
      asset("a", { Cloud: { provider: "Fixture", download: "Stored" } }),
    ]),
  ],
  [
    "cloud-downloading",
    projection([
      {
        ...asset("a", {
          Cloud: { provider: "Fixture", download: "Downloading" },
        }),
        focused: true,
      },
    ]),
  ],
  [
    "cloud-error",
    projection([
      asset("a", {
        Cloud: {
          provider: "Fixture",
          download: { Failed: "Synthetic failure" },
        },
      }),
    ]),
  ],
  ["drag-source", drag],
  ["external-drop", dropping],
  ["folder-drop", folderDrop],
  ["rename", rename],
  ["60000-items", huge],
];
for (const [state, view, extra] of cases)
  test(`${state}: op tree, real validator and AX projection`, async () => {
    const { root, commits } = recorder();
    await root.render(
      <ProjectView
        view={view}
        input={() => {}}
        menu={() => {}}
        page={() => {}}
        native={native}
        layout={layout}
        {...extra}
      />,
    );
    const reports = replay(commits, view);
    for (const report of reports) {
      expect(report.accepted).toBe(true);
      expect(report.diagnostics).toEqual([]);
      expect(report.wireDiagnostics).toEqual([]);
    }
    const report = reports.at(-1)!;
    expect({
      tree: treeSnapshot(report.nodes),
      ax: report.accessibility,
      focusOrder: report.focusOrder,
    }).toMatchSnapshot();
    expect(report.accessibility.length).toBe(Object.keys(report.nodes).length);
    const grid = report.accessibility.find(
      (n: any) => n.label === "Project media",
    );
    expect(grid.role).toBe("List");
    if (view) {
      const cells = report.accessibility.filter((n: any) => n.role === "Cell");
      expect(cells.length).toBe(view.page.rows.length);
      expect(grid.rowCount).toBe(view.page.total);
      expect(grid.multiselectable).toBe(true);
    }
    if (state === "60000-items") {
      expect(Object.keys(report.nodes).length).toBeLessThan(200);
      expect(find(root, "Project media").props.attr).toEqual({
        rowCount: 60_000,
        multiselectable: true,
      });
    }
    await root.dispose();
  });
test("hover actions appear, never become Tab stops and disappear during a drag", async () => {
  const { root, commits } = recorder();
  let view = projection([asset()]);
  const render = () =>
    root.render(
      <ProjectView
        view={view}
        input={() => {}}
        menu={() => {}}
        page={() => {}}
        native={native}
        layout={layout}
      />,
    );
  await render();
  expect(find(root, "Show in Finder")).toBeUndefined();
  await event(root, card(root, "a").id, "pointermove");
  const reports = replay(commits, view);
  const report = reports.at(-1)!;
  expect(find(root, "Show in Finder").props.tabIndex).toBe(-1);
  expect({
    tree: treeSnapshot(report.nodes),
    ax: report.accessibility,
    focusOrder: report.focusOrder,
  }).toMatchSnapshot("hover");
  view = { ...view, drag: drag.drag };
  await render();
  expect(find(root, "Show in Finder")).toBeUndefined();
  await root.dispose();
});
test("search Escape clears, then closes; no-results copy uses exact query", async () => {
  const { root, commits } = recorder();
  const inputs: Input[] = [];
  let view = projection([asset()]);
  const render = () =>
    root.render(
      <ProjectView
        view={view}
        input={(i) => inputs.push(i)}
        menu={() => {}}
        page={() => {}}
        native={native}
        layout={layout}
      />,
    );
  await render();
  await event(root, find(root, "Search Media").id, "click");
  await event(root, find(root, "Search media").id, "input", {
    value: "not here",
    composing: false,
  });
  expect(inputs.at(-1)).toEqual({
    Filter: { query: "not here", kind: null, missing_only: false, proxy: null },
  });
  view = projection();
  await render();
  const report = replay(commits, view).at(-1)!;
  expect({
    tree: treeSnapshot(report.nodes),
    ax: report.accessibility,
  }).toMatchSnapshot("no search results");
  await event(root, find(root, "Search media").id, "keydown", {
    key: "Escape",
  });
  expect(find(root, "Search media").props.value).toBe("");
  await event(root, find(root, "Search media").id, "keydown", {
    key: "Escape",
  });
  expect(find(root, "Search media")).toBeUndefined();
  await root.dispose();
});
test("one grid Tab stop; header, search, crumbs, grid, selection order", async () => {
  const { root, commits } = recorder();
  const view = projection(multi);
  view.folder = "f";
  view.crumbs = [{ id: "f", name: "Shots", parent: null, items: 2 }];
  await root.render(
    <ProjectView
      view={view}
      input={() => {}}
      menu={() => {}}
      page={() => {}}
      native={native}
      layout={layout}
    />,
  );
  await event(root, find(root, "Search Media").id, "click");
  const r = replay(commits, view).at(-1)!;
  expect(r.focusOrder.map((id: number) => r.nodes[id].props.label)).toEqual([
    "Project",
    "Search Media",
    "Add",
    "Search media",
    "Media",
    "Project media",
    "Delete selected media",
  ]);
  expect(
    root
      .inspect()
      .nodes.filter((n) => n.props.role === "cell")
      .every((n) => n.props.tabIndex === -1),
  ).toBe(true);
  await root.dispose();
});
test("native row windows request only their item window", async () => {
  const pages: unknown[] = [];
  const { root } = recorder();
  await root.render(
    <ProjectView
      view={huge}
      input={() => {}}
      menu={() => {}}
      page={(...p) => pages.push(p)}
      native={native}
      layout={layout}
    />,
  );
  const virtual = root.inspect().nodes.find((n) => n.type === "virtual-list")!;
  expect(virtual.props.count).toBe(30_000);
  await event(root, virtual.id, "rangechange", { range: [100, 106] });
  expect(pages).toEqual([[200, 12]]);
  await event(root, virtual.id, "rangechange", { range: [0, 3000] });
  expect(pages.length).toBe(1);
  await root.dispose();
});
test("exact asset, composition and folder spoken values", () => {
  expect(accessibleName(asset())).toBe(
    "a.mov, video, 1920 by 1080, 12 seconds, 48 megabytes",
  );
  expect(accessibleName(comp())).toBe(
    "Launch Film, composition, 1920 by 1080, 20 seconds, open in the timeline",
  );
  expect(accessibleName(folder())).toBe("Shots, folder, 4 items");
  expect(accessibleName(selected)).toEndWith(", selected");
  expect(meta(asset())).toBe("MOV · 1920×1080 · 0:12 · 48 MB");
  const unknown = asset("x", "Unknown");
  if ("Asset" in unknown.data) {
    Object.assign(unknown.data.Asset[0], {
      width: null,
      height: null,
      duration: null,
      bytes: null,
      format: null,
    });
    expect(meta(unknown)).toBe("");
    unknown.data.Asset[1].availability = "Ready";
    expect(meta(unknown)).toBe("Ready to use");
  }
  expect(
    status(
      "Unknown",
      (asset().data as { Asset: [import("../contract").Asset, unknown] })
        .Asset[0],
    ),
  ).toBeNull();
});

test("native marquee driver emits model inputs; cards and nested actions keep ownership", async () => {
  const { root } = recorder();
  const inputs: Input[] = [];
  const driver = {
    ...native,
    marquee(
      phase: "begin" | "move" | "end" | "cancel",
      event: import("../../../src").SurfaceEvent,
      emit: (i: Input) => void,
    ) {
      if (phase === "begin")
        emit({
          BeginMarquee: {
            pointer: event.pointerId!,
            modifiers: {
              meta: false,
              control: false,
              alt: false,
              shift: false,
            },
          },
        });
      if (phase === "move")
        emit({
          MarqueeHits: {
            pointer: event.pointerId!,
            activated: true,
            ids: ["a"],
          },
        });
      if (phase === "end" || phase === "cancel")
        emit({ EndMarquee: { pointer: event.pointerId! } });
    },
  };
  await root.render(
    <ProjectView
      view={projection([asset()])}
      input={(i) => inputs.push(i)}
      menu={() => {}}
      page={() => {}}
      native={driver}
      layout={layout}
    />,
  );
  const grid = find(root, "Project media").id;
  await event(root, grid, "pointerdown", {
    pointerId: 7,
    x: 10,
    y: 10,
    button: 0,
  });
  await event(root, grid, "pointermove", { pointerId: 7, x: 90, y: 90 });
  await event(root, grid, "pointerup", { pointerId: 7 });
  expect(
    inputs.map((i) => (typeof i === "object" ? Object.keys(i)[0] : i)),
  ).toEqual(["BeginMarquee", "MarqueeHits", "EndMarquee"]);
  inputs.length = 0;
  await event(root, card(root, "a").id, "pointerdown", {
    pointerId: 7,
    button: 0,
  });
  expect(inputs.length).toBe(1);
  expect(inputs[0]).toHaveProperty("Pointer");
  await root.dispose();
});
test("F2 requests folder rename; a composing field cannot commit", async () => {
  const { root } = recorder();
  const inputs: Input[] = [];
  let view = projection([{ ...folder(), focused: true }]);
  const render = () =>
    root.render(
      <ProjectView
        view={view}
        input={(i) => inputs.push(i)}
        menu={() => {}}
        page={() => {}}
        native={native}
        layout={layout}
      />,
    );
  await render();
  await event(root, find(root, "Project media").id, "keydown", { key: "F2" });
  expect(inputs.at(-1)).toEqual({ BeginRename: "f" });
  view = { ...view, renaming: "f" };
  await render();
  const field = root
    .inspect()
    .nodes.find((n) => n.type === "input" && n.props.label === "Shots")!;
  await event(root, field.id, "keydown", { key: "Enter", composing: true });
  expect(inputs.length).toBe(1);
  await event(root, field.id, "keydown", { key: "Escape" });
  expect(inputs.at(-1)).toBe("CancelRename");
  await root.dispose();
});
test("menu copy, separators, shortcuts and eligibility match the spec", async () => {
  const { menuEntries } = await import("../presentation");
  expect(
    menuEntries([
      { command: "NewComposition", enabled: true },
      { command: { NewFolder: null }, enabled: true },
      { command: "ImportFiles", enabled: true },
      { command: { ImportFolder: null }, enabled: true },
    ]),
  ).toEqual([
    {
      command: "NewComposition",
      enabled: true,
      label: "New Composition…",
      separatorBefore: false,
      shortcut: "⌘N",
    },
    {
      command: { NewFolder: null },
      enabled: true,
      label: "New Folder",
      separatorBefore: false,
    },
    {
      command: "ImportFiles",
      enabled: true,
      label: "Import Media…",
      separatorBefore: true,
      shortcut: "⌘I",
    },
    {
      command: { ImportFolder: null },
      enabled: true,
      label: "Import Folder…",
      separatorBefore: false,
    },
  ]);
  expect(
    menuEntries(
      [
        { command: { Replace: "a" }, enabled: true },
        { command: "DeleteAssets", enabled: true },
      ],
      asset("a", "Offline"),
      2,
    ).map((e) => e.label),
  ).toEqual(["Locate File…", "Delete Selected Media…"]);
});
