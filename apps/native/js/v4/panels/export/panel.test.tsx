import { expect, test } from "bun:test";
import {
  createRoot,
  type Commit,
  type SurfaceRoot,
  type EventDetail,
} from "../../src";
import { fixture, initial } from "./fixtures";
import { ExportSheet } from "./panel";
import type { ExportSnapshot } from "./model";
import { replay } from "./wire";

async function mounted(seed = initial()) {
  const f = fixture(seed);
  f.model.open = true;
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  await root.render(<ExportSheet model={f.model} anchor={0} />);
  return { ...f, root, commits };
}
function named(root: SurfaceRoot, name: string) {
  const n = [...root.host.nodes.values()].find((n) => n.props.id === name);
  if (!n) throw new Error(`missing ${name}`);
  return n;
}
async function event(
  root: SurfaceRoot,
  id: number,
  type: string,
  detail: EventDetail = {},
) {
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: id,
      preventDefault: true,
      deliveries: [{ id, capture: false }],
    },
    type,
    detail,
  );
}
function check(rows: ReturnType<typeof replay>) {
  for (const row of rows) {
    expect(row.accepted).toBe(true);
    expect(row.diagnostics).toEqual([]);
    expect(row.wireDiagnostics).toEqual([]);
    expect(row.ax).toHaveLength(Object.keys(row.nodes).length);
  }
  return rows.at(-1)!;
}
function tree(row: ReturnType<typeof replay>[number], id = 0): unknown {
  const n = row.nodes[id];
  return {
    type: n.type,
    text: n.text,
    props: n.props,
    children: n.children.map((id: number) => tree(row, id)),
  };
}
const cases: [string, (s: ExportSnapshot) => void][] = [
  [
    "empty (no project, sheet absent)",
    (s) => {
      s.project = null;
    },
  ],
  ["selected video", () => {}],
  [
    "known capabilities (loading N/A)",
    (s) => {
      s.capabilities.mp4 = {
        available: false,
        reason: "H.264 MP4 export is unavailable on this Mac",
      };
    },
  ],
  [
    "disabled audio and unset work area",
    (s) => {
      s.project!.hasAudio = false;
      s.project!.hasWorkArea = false;
    },
  ],
  [
    "ProRes alpha",
    (s) => {
      s.options.format = "prores";
      s.options.alpha = true;
    },
  ],
  [
    "PNG sequence",
    (s) => {
      s.options.format = "png";
    },
  ],
  [
    "still",
    (s) => {
      s.options.format = "still";
      s.project!.summary = "1920×1080 · frame at 00:00:00:00";
    },
  ],
  [
    "project",
    (s) => {
      s.options.format = "json";
      s.project!.summary = "Launch Film.pmv";
    },
  ],
  [
    "no captions",
    (s) => {
      s.project!.captionLayers = 0;
    },
  ],
  [
    "multiple captions",
    (s) => {
      s.project!.captionLayers = 2;
    },
  ],
  [
    "custom size",
    (s) => {
      s.options.scale = "custom";
    },
  ],
  [
    "inline invalid size",
    (s) => {
      s.options.scale = "custom";
      s.options.width = 8193;
    },
  ],
  [
    "choosing destination (no progress)",
    (s) => {
      s.run = { phase: "choosingDestination" };
    },
  ],
  [
    "preparing",
    (s) => {
      s.run = { phase: "preparing" };
    },
  ],
  [
    "rendering with preview and ETA",
    (s) => {
      s.run = {
        phase: "rendering",
        frame: 120,
        total: 300,
        elapsed: 2,
        eta: 120,
        preview: "fixture:frame",
      };
    },
  ],
  [
    "finishing",
    (s) => {
      s.run = { phase: "finishing" };
    },
  ],
  [
    "cancelling",
    (s) => {
      s.run = { phase: "cancelling" };
    },
  ],
  [
    "failed",
    (s) => {
      s.run = {
        phase: "failed",
        message: "Reconnect the source and try again.",
      };
    },
  ],
  [
    "done (sheet absent)",
    (s) => {
      s.run = { phase: "done", path: "fixture.mp4" };
    },
  ],
  [
    "cancelled (sheet absent)",
    (s) => {
      s.run = { phase: "cancelled" };
    },
  ],
];
for (const [name, configure] of cases)
  test(`wire snapshot: ${name}`, async () => {
    const s = initial();
    configure(s);
    const f = await mounted(s);
    if (!f.commits.length) {
      expect(f.root.host.root.children).toHaveLength(0);
      expect([]).toMatchSnapshot();
    } else expect(tree(check(replay(f.commits)))).toMatchSnapshot();
    await f.root.dispose();
  });
test("hover/focus use system controls; op tree stays stable, with one selected kind stop", async () => {
  const f = await mounted();
  const before = check(replay(f.commits));
  const segment = [...f.root.host.nodes.values()].find(
    (n) => n.props.role === "radio" && n.props.label === "Video",
  )!;
  await event(f.root, segment.id, "pointermove");
  await event(f.root, segment.id, "focus");
  await f.root.flush();
  const after = check(replay(f.commits, 0, segment.id));
  expect(tree(after)).toEqual(tree(before));
  expect(tree(after)).toMatchSnapshot();
  const order = after.focusOrder.map((id: number) => after.nodes[id].props.id);
  expect(order.slice(1)).toEqual([
    "export.format",
    "export.scale",
    "export.fps",
    "export.range",
    "export.quality",
    "export.mblur",
    "export.audio",
    "export.captionsBurn",
    "export.captionsSidecar",
    "export.preset",
    "export.presetName",
    "export.cancel",
    "export.start",
  ]);
  expect(after.ax.find((n: any) => n.id === segment.id)).toMatchObject({
    role: "RadioButton",
    label: "Video",
    toggled: "Some(True)",
  });
  await f.root.dispose();
});
test("all 16 scheme/accessibility combinations pass real native validation", async () => {
  const f = await mounted();
  for (let appearance = 0; appearance < 16; appearance++)
    check(replay(f.commits, appearance));
  await f.root.dispose();
});
test("keyboard kind navigation and popup/checkbox interactions reach the model and host planner", async () => {
  const f = await mounted();
  const video = [...f.root.host.nodes.values()].find(
    (n) => n.props.role === "radio" && n.props.label === "Video",
  )!;
  await event(f.root, video.id, "keydown", { key: "ArrowRight" });
  await f.root.flush();
  // Worker command acknowledgements finish outside native input dispatch.
  await new Promise((r) => setTimeout(r, 0));
  await f.root.flush();
  expect(f.model.options.format).toBe("still");
  expect(named(f.root, "export.start").props.label).toBe("Export Frame");
  await event(f.root, named(f.root, "export.format").id, "change", {
    value: "png",
  });
  await new Promise((r) => setTimeout(r, 0));
  await f.root.flush();
  expect(f.model.options.format).toBe("png");
  await event(f.root, named(f.root, "export.alpha").id, "change", {
    value: true,
  });
  await new Promise((r) => setTimeout(r, 0));
  await f.root.flush();
  expect(f.model.options.alpha).toBe(true);
  check(replay(f.commits));
  await f.root.dispose();
});
test("Return in preset field saves without starting export; Save joins focus order when named", async () => {
  const f = await mounted();
  // The protocol's key filter has no modifier predicates. Suppressing '.' at
  // sheet scope would prevent ordinary periods in the preset name field.
  const sheet = named(f.root, "export.sheet");
  const listeners = f.commits
    .flatMap((c) => c.ops)
    .find((op) => op.op === "setListeners" && op.id === sheet.id);
  if (!listeners || listeners.op !== "setListeners")
    throw new Error("missing sheet listeners");
  expect(listeners.flags.keydown.keys).not.toContain(".");
  await event(f.root, named(f.root, "export.presetName").id, "input", {
    value: "Delivery",
  });
  await f.root.flush();
  let row = check(replay(f.commits));
  expect(
    row.focusOrder.map((id: number) => row.nodes[id].props.id).slice(-4),
  ).toEqual([
    "export.presetName",
    "export.savePreset",
    "export.cancel",
    "export.start",
  ]);
  await event(f.root, named(f.root, "export.presetName").id, "keydown", {
    key: "Enter",
  });
  await new Promise((r) => setTimeout(r, 0));
  await f.root.flush();
  expect(f.model.state.presets).toHaveLength(1);
  expect(f.model.state.run.phase).toBe("idle");
  row = check(replay(f.commits));
  expect(row.nodes[named(f.root, "export.preset").id].props.value).toBe(
    "fixture-preset-1",
  );
  await f.root.dispose();
});
test("accessibility exposes dialog, exact control labels, units and actual progress; unsupported announcements remain explicit gaps", async () => {
  const s = initial();
  s.options.scale = "custom";
  const f = await mounted(s);
  const row = check(replay(f.commits));
  expect(
    row.ax.find((n: any) => n.id === named(f.root, "export.sheet").id),
  ).toMatchObject({ role: "Dialog", label: "Export" });
  expect(
    row.ax.find((n: any) => n.id === named(f.root, "export.width").id),
  ).toMatchObject({ role: "NumberInput", label: "Width", value: "1920 pixels" });
  expect(
    row.ax.find((n: any) => n.id === named(f.root, "export.audio").id),
  ).toMatchObject({ role: "CheckBox", label: "Include Audio" });
  await f.root.dispose();
  s.run = { phase: "rendering", frame: 120, total: 300, elapsed: 2, eta: 120 };
  const g = await mounted(s);
  const progress = named(g.root, "export.progress");
  const native = check(replay(g.commits));
  expect(native.ax.find((n: any) => n.id === progress.id)).toMatchObject({
    role: "ProgressIndicator",
    label: "Export progress",
    numericValue: 0.4,
  });
  expect(progress.props.metadata).toEqual({
    export: { valueDescription: "40 percent, frame 120 of 300" },
  });
  await g.root.dispose();
});
test("drag/drop, multi-select and 60k rows are N/A; the sheet declares no list or drag listeners", async () => {
  const f = await mounted();
  expect(
    [...f.root.host.nodes.values()].some((n) =>
      ["virtual-list", "scroll"].includes(n.type),
    ),
  ).toBe(false);
  expect(JSON.stringify(f.commits)).not.toMatch(
    /dragstart|dragover|drop|multiselect/,
  );
  await f.root.dispose();
});

test("toolbar trigger has spec copy and disables without a project or during a job", async () => {
  const { ExportButton } = await import("./trigger");
  for (const [state, disabled, tooltip] of [
    [initial(), false, "Export (⌘E)"],
    [{ ...initial(), project: null }, true, "Export (⌘E)"],
    [
      { ...initial(), run: { phase: "preparing" } },
      true,
      "Export already running",
    ],
  ] as const) {
    const f = fixture(state as ExportSnapshot);
    const commits: Commit[] = [];
    const root = createRoot(async (c) => {
      commits.push(c);
      return { accepted: true, revision: c.baseRevision + 1 };
    });
    await root.render(<ExportButton model={f.model} />);
    expect(named(root, "export.toolbar").props).toMatchObject({
      label: "Export",
      disabled,
      title: tooltip,
    });
    check(replay(commits));
    await root.dispose();
  }
});
