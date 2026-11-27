import { expect, test } from "bun:test";
import type { InspectorSnapshot, Row } from "../contracts";
import { InspectorPanel, rowMenu } from "../view";
import { fixture, snapshot, binding, numberRow } from "./fixture";
import { Probe } from "./probe";

const cases = [
  "empty",
  "composition",
  "selected",
  "font-loading",
  "hover",
  "focus",
  "editing",
  "scrubbing",
  "animated",
  "key-at-playhead",
  "expression",
  "mixed",
  "partial-applicability",
  "locked",
  "disabled",
  "error",
  "drop",
  "playing",
  "60000-selected",
  "collapsed",
  "audio",
];
function state(name: string): InspectorSnapshot {
  const value = snapshot(),
    row = value.sections[0].rows[0];
  if (name === "empty") {
    value.target = { kind: "no-project" };
    value.sections = [];
  }
  if (name === "composition") {
    value.target = { kind: "composition", key: "C" };
    value.sections = [
      {
        id: "composition",
        title: "Composition",
        collapsed: false,
        rows: [
          numberRow("width", "Width"),
          numberRow("height", "Height"),
          numberRow("fps", "Frame Rate"),
          numberRow("duration", "Duration"),
          {
            id: "bg",
            label: "Background",
            control: {
              kind: "colour",
              binding: binding("bg"),
              value: "#F5F3EF",
            },
          },
        ],
      },
    ];
  }
  if (name === "font-loading")
    value.sections[0].rows = [
      {
        id: "font",
        label: "Font",
        control: {
          kind: "font",
          options: [],
          value: "",
          binding: binding("font"),
        },
      },
    ];
  if (name === "animated") row.animation = "animated";
  if (name === "key-at-playhead") row.animation = "key-at-playhead";
  if (name === "expression")
    row.expression = {
      binding: binding("expr"),
      source: "time * 2",
      error: null,
    };
  if (
    name === "mixed" ||
    name === "60000-selected" ||
    name === "partial-applicability"
  ) {
    if (value.target.kind === "layers") {
      value.target.count = name === "60000-selected" ? 60000 : 3;
      value.target.sharedType = null;
    }
    if (row.control.kind === "number") row.control.axes[0].mixed = true;
  }
  if (name === "disabled") row.disabled = true;
  if (name === "locked") {
    row.locked = true;
    if (value.target.kind === "layers") value.target.first.locked = true;
  }
  if (name === "error") row.error = { kind: "number" };
  if (name === "playing") value.playing = true;
  if (name === "collapsed") value.sections[0].collapsed = true;
  if (name === "audio" && value.target.kind === "layers") {
    value.target.first.audio = true;
    value.target.first.acceptsEffects = false;
    value.target.first.type = "audio";
    value.target.first.typeLabel = "Audio";
  }
  return value;
}
for (const name of cases)
  test(`spec §3: ${name} six-op tree, native validator and AX snapshot`, async () => {
    const f = fixture(state(name)),
      p = new Probe();
    await f.session.start();
    try {
      await p.render(<InspectorPanel session={f.session} />);
      let native = undefined;
      if (name === "drop")
        await p.drive("inspector:content", "dragover", {
          drag: {
            types: ["application/x-powermove-effect"],
            data: { type: "blur" },
            effect: "copy",
          },
        });
      if (name === "font-loading") await p.drive("value:font", "click");
      if (name === "hover")
        await p.drive("value:opacity:0", "pointermove", { x: 1, y: 1 });
      if (name === "focus") await p.drive("value:opacity:0", "focus");
      if (name === "editing")
        native = p
          .native(
            { control: { type: "focus", id: "value:opacity:0" } },
            { control: { type: "input", id: "value:opacity:0", text: "*2" } },
          )
          .at(-1)?.edits;
      if (name === "scrubbing")
        native = p
          .native(
            { scrub: { type: "begin", id: "value:opacity:0", x: 0, y: 0 } },
            { scrub: { type: "sample", x: 20, y: 0 } },
          )
          .at(-1)?.edits;
      expect(p.result.accepted).toBe(true);
      expect(p.result.diagnostics).toEqual([]);
      expect(p.result.wireDiagnostics).toEqual([]);
      expect(p.result.accessibility).toHaveLength(
        Object.keys(p.result.nodes).length,
      );
      // useId's prefix depends on the Preact root counter, not authored identity.
      const normalized = JSON.parse(
        JSON.stringify({
          ops: p.commits.flatMap((c) => c.ops),
          tree: p.result.nodes,
          ax: p.result.accessibility,
          focus: p.result.focusOrder,
          native,
          requests: f.native,
        }).replace(/P\d+-\d+/g, "generated-label"),
      );
      expect(
        [
          "ops",
          ...normalized.ops.map((op: unknown) => JSON.stringify(op)),
          "tree",
          ...Object.values(normalized.tree).map((node) => JSON.stringify(node)),
          "accessibility",
          ...normalized.ax.map((node: unknown) => JSON.stringify(node)),
          JSON.stringify({
            focus: normalized.focus,
            native: normalized.native,
            requests: normalized.requests,
          }),
        ].join("\n"),
      ).toMatchSnapshot();
    } finally {
      await p.dispose();
      await f.session.dispose();
    }
  });

test("Camera headings show the layer type when it adds context", async () => {
  for (const name of ["Camera", "Main Cam"]) {
    const s = snapshot();
    if (s.target.kind !== "layers") throw new Error("Expected a layer");
    Object.assign(s.target.first, {
      name,
      type: "extension",
      typeLabel: "Camera",
      acceptsEffects: false,
      studio: true,
    });
    const f = fixture(s);
    const p = new Probe();
    await f.session.start();
    try {
      await p.render(<InspectorPanel session={f.session} />);
      const heading = Object.values(p.result.nodes).find(
        (node) => node.props.role === "heading",
      )!;
      const text = heading.children.flatMap((id) =>
        p.result.nodes[id].children.map((child) => p.result.nodes[child].text),
      );
      expect(text).toEqual(
        name === "Camera" ? ["Camera"] : ["Main Cam", "· Camera"],
      );
      expect(heading.props.label).toBe(
        name === "Camera" ? "Camera layer" : "Main Cam, camera layer",
      );
    } finally {
      await p.dispose();
      await f.session.dispose();
    }
  }
});

test("selection identity, axes, order, unit actions and checkbox AX semantics", async () => {
  const s = snapshot();
  s.sections[0].rows = [
    {
      id: "position",
      label: "Position",
      animation: "static",
      control: {
        kind: "number",
        axes: [
          { axis: "X", binding: binding("x"), value: 960, step: 1, unit: "px" },
          { axis: "Y", binding: binding("y"), value: 540, step: 1, unit: "px" },
        ],
      },
    },
    { ...numberRow("scale", "Scale"), linked: true },
    {
      id: "visible",
      label: "Visible",
      control: {
        kind: "toggle",
        value: true,
        mixed: true,
        binding: binding("visible"),
      },
    },
    {
      id: "colour",
      label: "Color",
      control: {
        kind: "colour",
        value: "#F5F3EF",
        binding: binding("color"),
        hex: "#F5F3EF",
        hexBinding: binding("hex"),
      },
    },
  ];
  const f = fixture(s),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    const ax = (id: string) =>
      p.result.accessibility.find((node) => node.id === p.node(id).id)!;
    expect(ax("value:position:X")).toMatchObject({
      role: "NumberInput",
      label: "Position X",
      value: "960 pixels",
      step: 1,
      increment: true,
      decrement: true,
    });
    expect(ax("value:visible")).toMatchObject({
      role: "CheckBox",
      label: "Visible",
      toggled: "Mixed",
    });
    expect(ax("value:colour").role).toBe("ColorWell");
    const focus = p.result.focusOrder.map(
      (id) => p.result.nodes[id].props.id ?? p.result.nodes[id].props.label,
    );
    expect(focus).toEqual([
      "inspector:add-effect",
      "align:left",
      "Transform",
      "key:position",
      "value:position:X",
      "value:position:Y",
      "key:scale",
      "value:scale:0",
      "link:scale",
      "value:visible",
      "value:colour",
      "hex:colour",
    ]);
    await p.drive("key:position", "click", { altKey: true });
    await p.drive("key:scale", "keydown", { key: " " });
    await p.drive("link:scale", "click");
    await f.session.settled();
    expect(f.model.intents).toEqual([
      { type: "row-action", row: "position", action: "edit-expression" },
      { type: "key", row: "scale", removeAnimation: false },
      { type: "row-action", row: "scale", action: "link" },
    ]);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("camera Transform presents Position, Point of Interest and one editable Rotation", async () => {
  const s = snapshot();
  if (s.target.kind !== "layers") throw new Error("expected layer selection");
  s.target.first = {
    ...s.target.first,
    name: "Camera",
    type: "extension",
    typeLabel: "Camera",
  };
  const vector = (
    id: string,
    label: string,
    paths: string[],
    unit = "",
  ): Row => ({
    id,
    label,
    animation: "static",
    control: {
      kind: "number",
      axes: ["X", "Y", "Z"].map((axis, i) => ({
        axis,
        binding: binding(paths[i]),
        value: 0,
        step: 1,
        precision: 2,
        unit,
      })),
    },
  });
  s.sections[0].rows = [
    vector("position", "Position", ["position.x", "position.y", "position.z"]),
    vector("target", "Point of Interest", [
      "camera.targetX",
      "camera.targetY",
      "camera.targetZ",
    ]),
    vector(
      "rotation3d",
      "Rotation",
      ["rotation.x", "rotation.y", "rotation"],
      "°",
    ),
  ];
  s.sections.push({
    id: "3d.camera",
    title: "Camera",
    collapsed: false,
    rows: [numberRow("camera.fov", "Field of View")],
  });
  const f = fixture(s);
  const p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    const numbers = p.result.accessibility
      .filter((node) => node.role === "NumberInput")
      .map((node) => node.label);
    expect(numbers).toEqual([
      "Position X",
      "Position Y",
      "Position Z",
      "Point of Interest X",
      "Point of Interest Y",
      "Point of Interest Z",
      "Rotation X",
      "Rotation Y",
      "Rotation Z",
      "Field of View",
    ]);
    const labels = p.result.accessibility.map((node) => node.label);
    expect(labels).not.toContain("Auto-Orient");
    expect(labels).not.toContain("Shot");
    expect(labels).not.toContain("Render Quality");
    await p.drive("key:rotation3d", "click");
    await p.drive("key:rotation3d", "click", { altKey: true });
    await f.session.settled();
    expect(f.model.intents).toEqual([
      { type: "key", row: "rotation3d", removeAnimation: false },
      { type: "row-action", row: "rotation3d", action: "edit-expression" },
    ]);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("3D Scene shows Sky and ignores the legacy Set Up Studio action", async () => {
  const s = snapshot();
  s.target = { kind: "composition", key: "c" };
  s.sections = [
    {
      id: "3d.scene",
      title: "3D Scene",
      collapsed: false,
      action: "set-up-studio",
      rows: [
        {
          id: "3d.scene.sky",
          label: "Sky",
          control: {
            kind: "choice",
            value: "none",
            binding: binding("3d.scene.sky"),
            options: [{ v: "none", label: "None" }],
          },
        },
      ],
    },
  ];
  const f = fixture(s);
  const p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(p.node("value:3d.scene.sky").props.label).toBe("Sky");
    const labels = p.result.accessibility.map((node) => node.label);
    for (const label of ["Set Up Studio", "Shot", "Add Shot…", "Render Quality"]) {
      expect(labels).not.toContain(label);
    }
    expect(() => p.node("section-action:3d.scene")).toThrow();
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("header/menu anchors, alignment roving focus and exact effect destination", async () => {
  const f = fixture(),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(
      Object.values(p.result.nodes).some(
        (node) => node.props.id === "inspector:new-layer",
      ),
    ).toBe(false);
    await p.drive("inspector:add-effect", "click");
    await p.drive("align:left", "keydown", { key: "ArrowRight" });
    expect(
      Object.values(p.result.nodes).find(
        (node) => node.props.focusRequest === "align:hcenter",
      ),
    ).toBeDefined();
    expect(p.node("align:hcenter").props.tabIndex).toBe(0);
    await p.drive("align:hcenter", "keydown", { key: "Enter" });
    await p.drive("inspector:content", "drop", {
      drag: {
        types: ["application/x-powermove-effect"],
        data: { type: "blur" },
        effect: "copy",
      },
    });
    await f.session.settled();
    expect(f.native).toEqual([
      {
        type: "add-effect-menu",
        anchor: p.node("inspector:add-effect").id,
        target: "a",
      },
    ]);
    expect(f.model.intents).toEqual([
      { type: "align", alignment: "hcenter" },
      { type: "drop-effect", target: "a", effect: { type: "blur" } },
    ]);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("disabled callbacks, invalid drops, row context menu and disclosure remove descendants", async () => {
  const f = fixture(),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    await p.drive("row:opacity", "pointerdown", { button: 2 });
    expect(f.native[0]).toMatchObject({
      type: "row-menu",
      row: "opacity",
      items: rowMenu(numberRow()),
    });
    await p.drive("inspector:content", "drop", {
      drag: { types: ["text/plain"], data: {}, effect: "copy" },
    });
    expect(f.model.intents).toEqual([]);
    await p.drive(p.named("Transform").id, "click");
    expect(f.model.intents.at(-1)).toEqual({
      type: "collapse",
      section: "transform",
      collapsed: true,
    });
    f.model.value.sections[0].collapsed = true;
    f.publish();
    await f.session.settled();
    await p.flush();
    expect(
      Object.values(p.result.nodes).some(
        (node) => node.props.id === "key:opacity",
      ),
    ).toBe(false);
    expect(f.session.ready("opacity")).toBe(false);
    expect(
      p.result.accessibility.some((node) => node.label === "Keyframe, Opacity"),
    ).toBe(false);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("all supported kit field variants, expression/range errors and section actions validate", async () => {
  const s = snapshot();
  const rows: Row[] = [
    {
      id: "text",
      label: "Text",
      control: {
        kind: "text",
        value: "日本🙂",
        binding: binding("text"),
        multiline: true,
      },
    },
    {
      id: "choice",
      label: "Blend Mode",
      control: {
        kind: "choice",
        value: false,
        binding: binding("choice"),
        options: [
          { v: false, label: "None" },
          { v: null, label: "None" },
        ],
      },
    },
    {
      id: "fill",
      label: "Fill",
      control: {
        kind: "fill",
        value: { type: "linear", stops: [] },
        description: "Linear Gradient",
        binding: binding("fill"),
      },
    },
    {
      id: "align",
      label: "Align",
      control: {
        kind: "segmented",
        value: "left",
        binding: binding("align"),
        options: [
          { id: "left", label: "Left" },
          { id: "right", label: "Right" },
        ],
      },
    },
    {
      ...numberRow("point", "Position"),
      control: {
        kind: "point",
        axes: [{ axis: "X", value: 5, step: 1, binding: binding("point") }],
      },
    },
    {
      ...numberRow(),
      error: { kind: "range", min: 0, max: 100 },
      expression: {
        binding: binding("expr"),
        source: "broken",
        error: "Enter a number.",
      },
    },
  ];
  s.sections[0].rows = rows;
  s.sections[0].action = "mask";
  const f = fixture(s),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(p.result.diagnostics).toEqual([]);
    expect(p.result.wireDiagnostics).toEqual([]);
    await p.drive("value:fill", "click");
    await p.drive("pad:point", "click");
    await p.drive("section-action:transform", "click");
    await p.drive(p.named("Right").id, "click");
    expect(f.model.intents).toContainEqual({
      type: "set-value",
      row: "align",
      value: "right",
    });
    expect(f.native.map((request) => request.type)).toEqual([
      "fill-popover",
      "point-pad",
    ]);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("60000 selection renders the same bounded rows and preserves stable IDs on playback", async () => {
  const f = fixture(),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    const count = Object.keys(p.result.nodes).length,
      id = p.node("value:opacity:0").id;
    f.model.value = state("60000-selected");
    f.publish();
    await f.session.settled();
    await p.flush();
    expect(Object.keys(p.result.nodes).length).toBe(count);
    expect(p.node("value:opacity:0").id).toBe(id);
    f.model.value.playing = true;
    if (f.model.value.sections[0].rows[0].control.kind === "number")
      f.model.value.sections[0].rows[0].control.axes[0].value = 55;
    f.publish("time");
    f.clock.frame();
    await f.session.settled();
    await p.flush();
    expect(p.node("value:opacity:0").id).toBe(id);
    expect(p.node("value:opacity:0").props.value).toBe(55);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("the Time section's three actions send their section actions", async () => {
  const s = snapshot();
  s.sections.push({
    id: "time",
    title: "Time",
    collapsed: false,
    rows: [],
    actions: ["time-remap", "freeze-frame", "reverse"],
  });
  const f = fixture(s),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(p.result.diagnostics).toEqual([]);
    for (const action of ["time-remap", "freeze-frame", "reverse"] as const) {
      await p.drive(`section-action:time:${action}`, "click");
      expect(f.model.intents).toContainEqual({
        type: "section-action",
        section: "time",
        action,
      });
    }
    expect(p.named("Freeze Frame").id).toBe(
      p.node("section-action:time:freeze-frame").id,
    );
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});
