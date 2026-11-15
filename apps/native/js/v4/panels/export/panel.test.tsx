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

async function mounted(seed = initial(), queue = false) {
  const f = fixture(seed);
  f.model.open = true;
  if (queue) f.model.showQueue();
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
    "done (completion banner)",
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
  // Focus follows the output form, then advanced controls and the pinned actions.
  expect(
    order.filter((id: string) =>
      [
        "export.format",
        "export.quality",
        "export.range",
        "export.scale",
        "export.fps",
        "export.destination.choose",
      ].includes(id),
    ),
  ).toEqual([
    "export.format",
    "export.quality",
    "export.range",
    "export.scale",
    "export.fps",
    "export.destination.choose",
  ]);
  expect(order.slice(-3)).toEqual([
    "export.queue.open",
    "export.queue.add",
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
test("a kind's page shows on the click; its plan is asked for after", async () => {
  const f = await mounted();
  const project = [...f.root.host.nodes.values()].find(
    (n) => n.props.role === "radio" && n.props.label === "Project",
  )!;
  const asked = () =>
    f.requests.filter(
      (r) =>
        r.operation.type === "command" && r.operation.command === "export.plan",
    ).length;
  const before = asked();
  const shown = f.commits.length;
  await event(f.root, project.id, "click");
  await f.root.flush();
  // The Project page is committed before any planning starts.
  expect(f.model.kind).toBe("project");
  expect(f.commits.length).toBeGreaterThan(shown);
  expect(asked()).toBe(before);
  await new Promise((r) => setTimeout(r, 0));
  await f.root.flush();
  expect(asked()).toBe(before + 1);
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
  expect(named(f.root, "export.start").props.label).toBe("Export");
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
    row.focusOrder.map((id: number) => row.nodes[id].props.id).slice(-5),
  ).toEqual([
    "export.presetName",
    "export.savePreset",
    "export.queue.open",
    "export.queue.add",
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
  ).toMatchObject({
    role: "NumberInput",
    label: "Width",
    value: "1920 pixels",
  });
  expect(
    row.ax.find((n: any) => n.id === named(f.root, "export.audio").id),
  ).toMatchObject({ role: "Switch", label: "Include Audio" });
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
  expect(progress.props.metadata).toMatchObject({
    pmKit: { queueProgress: "bar" },
    export: { valueDescription: "40 percent, frame 120 of 300" },
  });
  await g.root.dispose();
});
test("drag/drop, multi-select and 60k rows are N/A; the sheet declares no list or drag listeners", async () => {
  const f = await mounted();
  const nodes = [...f.root.host.nodes.values()];
  expect(nodes.some((n) => n.type === "virtual-list")).toBe(false);
  // Only the option body scrolls (capped at metrics.sheet_body_max) so the
  // sheet and its footer stay on screen.
  expect(
    nodes.filter((n) => n.type === "scroll").map((n) => n.props.id),
  ).toEqual(["export.options"]);
  expect(JSON.stringify(f.commits)).not.toMatch(
    /dragstart|dragover|drop|multiselect/,
  );
  await f.root.dispose();
});

test("toolbar trigger opens progress during a job and disables without a project", async () => {
  const { ExportButton } = await import("./trigger");
  for (const [state, disabled, tooltip] of [
    [initial(), false, "Export (⌘E)"],
    [{ ...initial(), project: null }, true, "Export (⌘E)"],
    [
      { ...initial(), run: { phase: "preparing" } },
      false,
      "View export progress",
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
test("offline refusal offers Export Without Them; only the person's click sends it", async () => {
  const s = initial();
  s.run = {
    phase: "failed",
    message:
      "1 clip is offline: Interview. Relink the originals, or export without them.",
    offline: true,
  };
  const f = await mounted(s);
  const without = named(f.root, "export.without");
  expect(without.props.label).toBe("Export Without Them");
  // Try Again stays the default; leaving clips out is never the Return key.
  expect(
    (
      named(f.root, "export.retry").props.metadata as {
        pmKit: { buttonRole: string };
      }
    ).pmKit.buttonRole,
  ).toBe("default");
  expect(
    (without.props.metadata as { pmKit: { buttonRole: string } }).pmKit
      .buttonRole,
  ).toBe("normal");
  expect(
    f.requests.some((r) => JSON.stringify(r).includes("export.without")),
  ).toBe(false);
  await event(f.root, without.id, "click");
  await f.root.flush();
  expect(
    f.requests.some((r) => JSON.stringify(r).includes("export.without")),
  ).toBe(true);
  expect(f.model.state.run.phase).toBe("preparing");
  await f.root.dispose();
  // Any other failure has no such choice.
  const plain = initial();
  plain.run = { phase: "failed", message: "The disk is full." };
  const g = await mounted(plain);
  expect(
    [...g.root.host.nodes.values()].some(
      (n) => n.props.id === "export.without",
    ),
  ).toBe(false);
  await g.root.dispose();
});
test("a 3D export states its preset's promise, its render time and the 75% option", async () => {
  const s = initial();
  const project = s.project!;
  project.studio = {
    saved: "Production",
    preset: "production",
    estimate: "12–60 min",
    state: "measured",
  };
  project.summary = "300 frames · 10 s · 1920×1080 · 3D Production, 12–60 min";
  s.bindings.studioQuality = "fixture:export:studioQuality";
  s.bindings.studioReducedSize = "fixture:export:studioReducedSize";
  const f = await mounted(s);
  // The row's second line is the chosen preset's promise in plain words.
  const texts = [...f.root.host.nodes.values()].map((n) => n.textContent);
  expect(texts.some((t) => t.includes("about 3 s a frame"))).toBe(true);
  // The measured time has its own row and closes the footer line, semibold.
  expect(named(f.root, "export.studioTime").props.label).toBe(
    "Render time 12–60 min",
  );
  expect(named(f.root, "export.summary").props.label).toBe(project.summary);
  expect(
    (named(f.root, "export.studioTime").props.style as { fontWeight: unknown })
      .fontWeight,
  ).toEqual({ token: "typography.display_weight" });
  // Render at 75% Size is offered, and off unless chosen.
  expect(named(f.root, "export.studioReducedSize").props.value).toBe(false);
  await f.root.dispose();
});

test("a QuickTime export offers color tag, start timecode and audio format, each where it applies", async () => {
  const seed = initial();
  seed.options.format = "prores";
  seed.options.profile = "422hq";
  const f = await mounted(seed);
  const has = (id: string) =>
    [...f.root.host.nodes.values()].some((n) => n.props.id === id);
  for (const id of ["export.colorTag", "export.timecode", "export.audioCoding"])
    expect(has(id)).toBe(true);
  expect(named(f.root, "export.colorTag").props.label).toBe("Color Tag");
  expect(named(f.root, "export.timecode").props.label).toBe("Start Timecode");
  expect(named(f.root, "export.audioCoding").props.label).toBe("Audio Format");
  const settle = async () => {
    await new Promise((r) => setTimeout(r, 0));
    await f.root.flush();
  };
  await event(f.root, named(f.root, "export.colorTag").id, "change", {
    value: "rec709",
  });
  await settle();
  expect(f.model.options.colorTag).toBe("rec709");
  await event(f.root, named(f.root, "export.timecode").id, "input", {
    value: " 01:00:00:00 ",
  });
  await settle();
  expect(f.model.options.timecode).toBe("01:00:00:00");
  await event(f.root, named(f.root, "export.audioCoding").id, "change", {
    value: "pcmFloat",
  });
  await settle();
  expect(f.model.options.audioCoding).toBe("pcmFloat");
  // The audio format belongs to the sound: off, it leaves the sheet.
  await event(f.root, named(f.root, "export.audio").id, "change", {
    value: false,
  });
  await settle();
  expect(has("export.audioCoding")).toBe(false);
  // MP4 keeps the color tag; a timecode track and PCM are QuickTime's.
  await event(f.root, named(f.root, "export.format").id, "change", {
    value: "mp4",
  });
  await settle();
  expect(has("export.colorTag")).toBe(true);
  expect(has("export.timecode")).toBe(false);
  expect(has("export.audioCoding")).toBe(false);
  check(replay(f.commits));
  await f.root.dispose();
});
test("HDR delivery carries its own label: no color tag row", async () => {
  const seed = initial();
  seed.project!.hdr = true;
  seed.options.format = "prores";
  seed.options.dynamicRange = "pq";
  const f = await mounted(seed);
  const has = (id: string) =>
    [...f.root.host.nodes.values()].some((n) => n.props.id === id);
  expect(has("export.dynamicRange")).toBe(true);
  expect(has("export.colorTag")).toBe(false);
  await f.root.dispose();
});
test("the dynamic range opens on what the host chose: the project's Output", async () => {
  // The host opens the sheet with the project's Output as the dynamic range
  // (builtins models.rs export_snapshot); the row shows it chosen.
  const seed = initial();
  seed.project!.hdr = true;
  seed.options.format = "mp4";
  seed.options.dynamicRange = "hlg";
  const f = await mounted(seed);
  expect(f.model.options.dynamicRange).toBe("hlg");
  expect(named(f.root, "export.dynamicRange").props.value).toBe("hlg");
  await f.root.dispose();
});

test("queue shows progress, pending controls, failed retry and completed Finder action", async () => {
  const seed = initial();
  seed.queue = [
    {
      id: 1,
      name: "Opening",
      format: "mp4",
      path: "/Exports/Opening.mp4",
      status: "Rendering",
      progress: 0.42,
      error: "",
    },
    {
      id: 2,
      name: "Logo",
      format: "prores",
      path: "/Exports/Logo.mov",
      status: "Pending",
      progress: 0,
      error: "",
    },
    {
      id: 3,
      name: "Yesterday",
      format: "mp4",
      path: "/Exports/Yesterday.mp4",
      status: "Done",
      progress: 1,
      error: "",
    },
    {
      id: 4,
      name: "Credits",
      format: "mp4",
      path: "/Exports/Credits.mp4",
      status: "Failed",
      progress: 0,
      error: "Missing media",
    },
  ];
  const f = await mounted(seed, true);
  expect(named(f.root, "queue.cancel.1")).toBeDefined();
  expect(named(f.root, "queue.cancel.2")).toBeDefined();
  expect(named(f.root, "queue.finder.3")).toBeDefined();
  expect(named(f.root, "queue.retry.4")).toBeDefined();
  expect(
    [...f.root.host.nodes.values()].some(
      (n) => n.props.id === "queue.remove.1",
    ),
  ).toBe(false);
  check(replay(f.commits));
  await f.root.dispose();
});

test("each queue status has exactly its inline actions, revealed on hover and keyboard focus", async () => {
  const seed = initial();
  seed.run = { phase: "rendering", frame: 42, total: 100, elapsed: 2 };
  seed.queue = (
    ["Pending", "Rendering", "Done", "Failed", "Cancelled"] as const
  ).map((status, index) => ({
    id: index + 1,
    name: `Job ${index + 1}`,
    format: "mp4",
    path: `/Exports/Job-${index + 1}.mp4`,
    status,
    progress: 0.42,
    error: "",
    finishedAt:
      status === "Done" || status === "Failed" || status === "Cancelled"
        ? 1700000000 + index
        : 0,
  }));
  const f = await mounted(seed, true);
  expect(named(f.root, "queue.time.5").textContent).not.toBe("");
  const expected = [
    ["cancel"],
    ["cancel"],
    ["finder", "remove"],
    ["retry", "remove"],
    ["remove"],
  ];
  for (const [index, actions] of expected.entries()) {
    const id = index + 1;
    const jobRow = named(f.root, `queue.job.${id}`);
    expect(jobRow.props.style).toMatchObject({
      height: { token: "metrics.export_job_row" },
    });
    for (const action of ["cancel", "retry", "finder", "remove"]) {
      const node = [...f.root.host.nodes.values()].find(
        (n) => n.props.id === `queue.${action}.${id}`,
      );
      expect(!!node).toBe(actions.includes(action));
      if (node) {
        expect(node.props.title).toBeTruthy();
        expect(node.props.label).toContain(`Job ${id}`);
        expect(node.parent?.props.style).toMatchObject({ opacity: 0 });
      }
    }
    await event(f.root, jobRow.id, "pointerenter");
    await f.root.flush();
    expect(
      named(f.root, `queue.${actions[0]}.${id}`).parent?.props.style,
    ).toMatchObject({ opacity: 1 });
    await event(f.root, jobRow.id, "pointerleave");
    await event(f.root, jobRow.id, "focus");
    await f.root.flush();
    expect(
      named(f.root, `queue.${actions[0]}.${id}`).parent?.props.style,
    ).toMatchObject({ opacity: 1 });
    await event(f.root, jobRow.id, "blur", {
      value: `queue.${actions.at(-1)}.${id}`,
    });
    await f.root.flush();
    expect(
      named(f.root, `queue.${actions[0]}.${id}`).parent?.props.style,
    ).toMatchObject({ opacity: 1 });
    await event(f.root, jobRow.id, "blur", { value: "export.cancel" });
    await f.root.flush();
    expect(
      named(f.root, `queue.${actions[0]}.${id}`).parent?.props.style,
    ).toMatchObject({ opacity: 0 });
  }
  check(replay(f.commits));
  await f.root.dispose();
});

test("project-file settings keep the output form above slim progress", async () => {
  const seed = initial();
  seed.options.format = "json";
  seed.run = { phase: "preparing" };
  const f = await mounted(seed);
  expect(
    [...f.root.host.nodes.values()].filter(
      (n) => n.props.id === "export.queue",
    ),
  ).toHaveLength(0);
  expect(named(f.root, "export.format")).toBeDefined();
  expect(named(f.root, "export.current")).toBeDefined();
  check(replay(f.commits));
  await f.root.dispose();
});

function has(root: SurfaceRoot, id: string) {
  return [...root.host.nodes.values()].some((node) => node.props.id === id);
}
function inside(root: SurfaceRoot, child: string, ancestor: string) {
  for (let node = named(root, child).parent; node; node = node.parent)
    if (node.props.id === ancestor) return true;
  return false;
}
const queueFixture = (): NonNullable<ExportSnapshot["queue"]> => [
  {
    id: 1,
    name: "Opening",
    format: "mp4",
    path: "/Exports/Opening.mp4",
    status: "Rendering",
    progress: 0.42,
    error: "",
  },
  {
    id: 2,
    name: "Logo",
    format: "prores",
    profile: "422",
    path: "/Exports/Logo.mov",
    status: "Pending",
    progress: 0,
    error: "",
  },
  {
    id: 3,
    name: "Finished",
    format: "mp4",
    path: "/Exports/Finished.mp4",
    status: "Done",
    progress: 1,
    finishedAt: 1700000003,
    error: "",
  },
];
test("settings open first with grouped output in order and actions outside the scroll", async () => {
  const seed = initial();
  seed.queue = queueFixture();
  const f = await mounted(seed);
  expect(has(f.root, "export.queue.rows")).toBe(false);
  expect(has(f.root, "queue.job.1")).toBe(false);
  const ids = [...f.root.host.nodes.values()].map((n) => n.props.id);
  const fields = [
    "export.format",
    "export.quality",
    "export.range",
    "export.scale",
    "export.fps",
    "export.destination.choose",
  ];
  const positions = fields.map((id) => ids.indexOf(id));
  expect(
    positions.every((v, i) => v >= 0 && (!i || v > positions[i - 1])),
  ).toBe(true);
  for (const id of fields)
    expect(inside(f.root, id, "export.options")).toBe(true);
  expect(named(f.root, "export.queue.badge").textContent).toBe("3");
  for (const id of ["export.queue.open", "export.queue.add", "export.start"]) {
    expect(inside(f.root, id, "export.footer")).toBe(true);
    expect(inside(f.root, id, "export.options")).toBe(false);
  }
  expect(named(f.root, "export.footer").props.style).toMatchObject({
    flexShrink: 0,
  });
  expect(named(f.root, "export.start").props.metadata).toMatchObject({
    pmKit: { buttonRole: "default" },
  });
  expect(named(f.root, "export.queue.add").props.metadata).toMatchObject({
    pmKit: { buttonRole: "normal" },
  });
  check(replay(f.commits));
  await f.root.dispose();
});
test("Queue pushes inside the same popover; Back and Escape return to settings", async () => {
  const seed = initial();
  seed.queue = queueFixture();
  const f = await mounted(seed);
  const sheet = named(f.root, "export.sheet").id;
  await event(f.root, named(f.root, "export.queue.open").id, "click");
  await f.root.flush();
  expect(named(f.root, "export.sheet").id).toBe(sheet);
  expect(has(f.root, "export.options")).toBe(false);
  expect(has(f.root, "queue.job.1")).toBe(true);
  const before = f.requests.length;
  expect(await f.model.key("Enter")).toBe(false);
  expect(f.requests.length).toBe(before);
  await event(f.root, named(f.root, "export.queue.back").id, "click");
  await f.root.flush();
  expect(has(f.root, "export.format")).toBe(true);
  expect(has(f.root, "export.queue.rows")).toBe(false);
  await event(f.root, named(f.root, "export.queue.open").id, "click");
  await f.root.flush();
  expect(await f.model.key("Escape")).toBe(true);
  await f.root.flush();
  expect(f.model.open).toBe(true);
  expect(has(f.root, "export.format")).toBe(true);
  check(replay(f.commits));
  await f.root.dispose();
});
test("empty Queue and History sections are hidden; both empty show one compact message", async () => {
  for (const jobs of [
    [],
    queueFixture().slice(0, 2),
    queueFixture().slice(2),
  ]) {
    const seed = initial();
    seed.queue = jobs;
    const f = await mounted(seed, true);
    const groups = [...f.root.host.nodes.values()]
      .filter((n) => n.props.role === "group")
      .map((n) => n.props.label);
    expect(groups.includes("Queue")).toBe(
      jobs.some((j) => j.status === "Pending" || j.status === "Rendering"),
    );
    expect(groups.includes("History")).toBe(
      jobs.some((j) => j.status === "Done"),
    );
    expect(has(f.root, "export.queue.empty")).toBe(!jobs.length);
    if (!jobs.length)
      expect(named(f.root, "export.queue.empty").props.label).toBe(
        "No exports yet",
      );
    check(replay(f.commits));
    await f.root.dispose();
  }
});
test("settings show one slim progress row above the fixed footer, then the completion banner", async () => {
  const seed = initial();
  seed.queue = queueFixture();
  seed.run = { phase: "rendering", frame: 42, total: 100, elapsed: 2 };
  const f = await mounted(seed);
  expect(named(f.root, "export.current.name").textContent).toBe("Opening");
  expect(named(f.root, "export.current.percent").textContent).toBe("42 %");
  expect(named(f.root, "export.progress").props.style).toMatchObject({
    height: { token: "metrics.export_job_progress_height" },
  });
  expect(has(f.root, "export.queue.indicator")).toBe(true);
  expect(has(f.root, "queue.job.1")).toBe(false);
  expect(inside(f.root, "export.current", "export.options")).toBe(false);
  const children = named(f.root, "export.sheet").children.map(
    (n) => n.props.id,
  );
  expect(children.indexOf("export.current")).toBeLessThan(
    children.indexOf("export.footer"),
  );
  await event(f.root, named(f.root, "export.cancel").id, "click");
  await f.root.flush();
  expect(
    f.requests.some(
      (r) =>
        r.operation.type === "command" &&
        r.operation.command === "export.cancel",
    ),
  ).toBe(true);
  await f.root.dispose();
  const done = initial();
  done.run = { phase: "done", path: "/Exports/Opening.mp4" };
  const g = await mounted(done);
  expect(has(g.root, "export.current")).toBe(false);
  expect(has(g.root, "export.completed")).toBe(true);
  expect(has(g.root, "export.showInFinder")).toBe(true);
  check(replay(g.commits));
  await g.root.dispose();
});
test("footer routes Add to Queue and Export to their existing commands", async () => {
  for (const [id, command] of [
    ["export.queue.add", "export.queue.add"],
    ["export.start", "export.begin"],
  ]) {
    const f = await mounted();
    await event(f.root, named(f.root, id).id, "click");
    await f.root.flush();
    expect(
      f.requests.some(
        (r) =>
          r.operation.type === "command" && r.operation.command === command,
      ),
    ).toBe(true);
    await f.root.dispose();
  }
});
test("Choose shows only the destination filename with the full path as its tooltip", async () => {
  const f = await mounted();
  await event(f.root, named(f.root, "export.destination.choose").id, "click");
  await f.root.flush();
  expect(named(f.root, "export.destination").textContent).toBe("Chosen.mp4");
  expect(named(f.root, "export.destination").props.title).toBe(
    "/Exports/Chosen.mp4",
  );
  expect(f.model.state.run.phase).toBe("idle");
  check(replay(f.commits));
  await f.root.dispose();
});
