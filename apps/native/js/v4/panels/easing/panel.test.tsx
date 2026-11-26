import { expect, test } from "bun:test";
import { recorder } from "../../test/helpers";
import { EasingModel, type EasingState } from "./model";
import { EasingPanel } from "./panel";

async function fixture(count = 0) {
  let state: EasingState = {
    open: true,
    count,
    presets: [{ id: "easeOut", curve: [0, 0, 0.58, 1] }],
  };
  const actions: unknown[] = [];
  const model = new EasingModel(
    () => state,
    async (a) => {
      actions.push(a);
    },
    (e) => {
      throw e;
    },
  );
  model.refresh();
  const { root, commits } = recorder();
  await root.render(
    <EasingPanel
      model={model}
      bindings={{ curve: "c", outX: "x", inX: "i" }}
    />,
  );
  const node = (id: string) =>
    [...root.host.nodes.values()].find((n) => n.props.id === id)!;
  const fire = async (
    id: string,
    type: string,
    detail: { key?: string; value?: string | number | number[] } = {},
  ) => {
    const n = node(id);
    await root.dispatch(
      {
        revision: root.inspect().revision,
        target: n.id,
        preventDefault: false,
        deliveries: [{ id: n.id, capture: false }],
      },
      type,
      detail,
    );
    await root.flush();
  };
  return {
    root,
    commits,
    actions,
    node,
    fire,
    select: async (count: number) => {
      state = { ...state, count };
      model.refresh();
      await root.flush();
    },
    refresh: async (patch: Partial<EasingState>) => {
      state = { ...state, ...patch };
      model.refresh();
      await root.flush();
    },
    settle: () => model.settled(),
  };
}
test("all twelve toolbar buttons publish symbols the native renderer reads", async () => {
  const f = await fixture();
  const symbols = {
    more: "ellipsis",
    copy: "square.on.square",
    paste: "point.topleft.down.to.point.bottomright.curvepath",
    easy: "clock.fill",
    undo: "arrow.uturn.backward",
    redo: "arrow.uturn.forward",
    coordinates: "slider.horizontal.3",
    mirror: "arrow.clockwise",
    "save-preset": "plus",
    "auto-apply": "bolt.fill",
    apply: "return",
    "add-group": "plus",
  };
  for (const [id, symbol] of Object.entries(symbols)) {
    const { metadata } = f.node(`easing:${id}`).props;
    expect(metadata).toHaveProperty("symbol", symbol);
    expect(metadata).not.toHaveProperty("pmKit.symbol");
  }
  expect(f.node("easing:auto-apply").props.style).toMatchObject({
    color: { token: "colors.text" },
    background: { token: "colors.selected_fill" },
  });
  await f.root.dispose();
});

test("preset group tabs center their label alongside the add button", async () => {
  const f = await fixture();
  const tab = f.node("easing:group:default");
  const add = f.node("easing:add-group");
  expect(tab.props.label).toBe("1");
  expect(tab.parent).toBe(add.parent);
  expect(tab.props.style).toMatchObject({
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    height: add.props.style?.height,
  });
  await f.root.dispose();
});

test("the dock can scroll the full panel body as selection changes", async () => {
  const f = await fixture();
  const layout = () =>
    [...f.root.host.nodes.values()].map((n) => [
      n.id,
      Object.fromEntries(
        Object.entries(n.props.style ?? {}).filter(([key]) => key !== "color"),
      ),
    ]);
  const before = layout();
  const body = f.node("easing:content");
  expect(body.props.style).toMatchObject({
    flexGrow: 1,
    flexShrink: 0,
    paddingBottom: { token: "metrics.inset" },
  });
  expect(body.props.style).not.toHaveProperty("overflowY");
  expect([...f.root.host.nodes.values()].some((n) => n.type === "scroll")).toBe(
    false,
  );
  expect(f.node("easing:preset:easeOut").parent?.parent).toBe(body);
  expect(body.children.at(-1)).toBe(f.node("easing:status"));
  expect(f.node("easing:apply").props.disabled).toBe(true);
  expect(f.node("easing:influence:outgoing")).toBeUndefined();
  await f.select(1);
  expect(f.node("easing:apply").props.disabled).toBe(false);
  expect(layout()).toEqual(before);
  await f.select(5);
  expect(layout()).toEqual(before);
  await f.root.dispose();
});
test("presets load into the editable graph with no selection", async () => {
  const f = await fixture();
  expect(f.node("easing:curve").props.disabled).not.toBe(true);
  await f.fire("easing:preset:easeOut", "click");
  expect(f.actions).toEqual([]);
  expect(f.node("easing:curve").props.value).toEqual([0, 0, 0.58, 1]);
  expect(f.node("easing:status").props.label).toBe(
    "Select keyframes to apply easing",
  );
  await f.select(1);
  await f.fire("easing:apply", "click");
  await f.settle();
  expect(f.actions).toEqual([{ type: "curve", curve: [0, 0, 0.58, 1] }]);
  await f.root.dispose();
});
test("hover never applies; click sends one action and selects the matching tile", async () => {
  const f = await fixture(2);
  await f.fire("easing:preset:easeOut", "pointerenter");
  expect(f.actions).toEqual([]);
  expect(f.node("easing:status").props.label).toBe("Ease Out · 0% / 42%");
  expect(
    (f.node("easing:curve").props.metadata as { pmKit: { preview?: number[] } })
      .pmKit.preview,
  ).toEqual([0, 0, 0.58, 1]);
  await f.fire("easing:preset:easeOut", "pointerleave");
  expect(
    (f.node("easing:curve").props.metadata as { pmKit: { preview?: number[] } })
      .pmKit.preview,
  ).toBeUndefined();
  await f.fire("easing:preset:easeOut", "click");
  await f.settle();
  await f.root.flush();
  expect(f.actions).toEqual([{ type: "preset", preset: "easeOut" }]);
  expect(f.node("easing:preset:easeOut").props["aria-selected"]).toBe(true);
  await f.root.dispose();
});

test("native surface validator accepts selection, preview and apply publications", async () => {
  const f = await fixture();
  await f.select(2);
  await f.fire("easing:preset:easeOut", "pointerenter");
  await f.fire("easing:preset:easeOut", "click");
  await f.settle();
  await f.root.flush();
  await f.fire("easing:more", "click");
  await f.fire("easing:coordinates", "click");
  await f.fire("easing:more", "click");
  await f.fire("easing:save-preset", "click");
  await f.fire("easing:add-group", "click");
  await f.refresh({
    library: {
      stateRevision: "1",
      source: {
        version: 1,
        active: "fav",
        groups: [{ id: "fav", name: "Favorites", presets: [] }],
      },
    },
  });
  await f.fire("easing:group:default", "click");
  const validator = process.env.PM_SURFACE_WIRE;
  if (!validator)
    throw new Error("Set PM_SURFACE_WIRE to a prebuilt native validator");
  const result = Bun.spawnSync([validator], {
    stdin: Buffer.from(
      f.commits.map((c) => JSON.stringify(c)).join("\n") + "\n",
    ),
  });
  expect(result.exitCode).toBe(0);
  const rows = result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  expect(rows.filter((r) => !r.accepted)).toEqual([]);
  expect(rows).toHaveLength(f.commits.length);
  await f.root.dispose();
});

test("interpolation and matching curve read back from the native selection", async () => {
  const f = await fixture(2);
  await f.fire("easing:coordinates", "click");
  await f.refresh({ interpolation: "hold", curve: [0, 0, 1, 1] });
  expect(f.node("easing:interpolation").props.value).toBe("hold");
  await f.refresh({ interpolation: "continuous", curve: [0, 0, 0.58, 1] });
  expect(f.node("easing:interpolation").props.value).toBe("continuous");
  expect(f.node("easing:preset:easeOut").props["aria-selected"]).toBe(true);
  expect(f.actions).toEqual([]);
  await f.root.dispose();
});

test("preset keyboard activation uses Enter and leaves Space to transport", async () => {
  const f = await fixture(2);
  await f.fire("easing:preset:easeOut", "keydown", { key: " " });
  await f.settle();
  expect(f.actions).toEqual([]);
  await f.fire("easing:preset:easeOut", "keydown", { key: "Enter" });
  await f.settle();
  expect(f.actions).toEqual([{ type: "preset", preset: "easeOut" }]);
  await f.root.dispose();
});

test("curve and influence drags preview locally and commit once", async () => {
  const f = await fixture(2);
  await f.fire("easing:curve", "input", { value: [0.2, 0.1, 0.8, 0.9] });
  await f.fire("easing:curve", "input", { value: [0.3, 0.1, 0.8, 0.9] });
  expect(f.node("easing:curve").props.value).toEqual([0.3, 0.1, 0.8, 0.9]);
  expect(f.actions).toEqual([]);
  await f.fire("easing:curve", "change", { value: [0.3, 0.1, 0.8, 0.9] });
  await f.settle();
  expect(f.actions).toEqual([{ type: "curve", curve: [0.3, 0.1, 0.8, 0.9] }]);
  const n = await fixture(2);
  await n.fire("easing:coordinates", "click");
  await n.fire("easing:influence:outgoing", "input", { value: "50" });
  await n.fire("easing:influence:outgoing", "input", { value: "70" });
  expect(n.actions).toEqual([]);
  expect(n.node("easing:curve").props.value).toEqual([0.7, 0, 1, 1]);
  await n.fire("easing:influence:outgoing", "change", { value: 70 });
  await n.settle();
  expect(n.actions).toEqual([{ type: "curve", curve: [0.7, 0, 1, 1] }]);
  await f.root.dispose();
  await n.root.dispose();
});

test("manual drag, mirror and undo stay local until Apply", async () => {
  const f = await fixture(2);
  await f.fire("easing:auto-apply", "click");
  await f.fire("easing:curve", "input", { value: [0.1, 0, 0.6, 1] });
  await f.fire("easing:curve", "input", { value: [0.2, 0, 0.6, 1] });
  await f.fire("easing:curve", "change", { value: [0.2, 0, 0.6, 1] });
  await f.refresh({ curve: [0, 0, 1, 1] });
  expect(f.node("easing:curve").props.value).toEqual([0.2, 0, 0.6, 1]);
  await f.fire("easing:undo", "click");
  expect(f.node("easing:curve").props.value).toEqual([0, 0, 1, 1]);
  expect(f.node("easing:undo").props.disabled).toBe(true);
  await f.fire("easing:redo", "click");
  await f.fire("easing:mirror", "click");
  expect(f.node("easing:curve").props.value).toEqual([0.4, 0, 0.8, 1]);
  expect(f.actions).toEqual([]);
  await f.fire("easing:apply", "click");
  await f.settle();
  expect(f.actions).toEqual([{ type: "curve", curve: [0.4, 0, 0.8, 1] }]);
  await f.root.dispose();
});

test("copy uses the selected keys and paste applies their curve to another selection", async () => {
  const f = await fixture(1);
  const from = [0.25, 0, 0.75, 1] as const;
  await f.refresh({ selection: [1, ["a"], []], curve: [...from] });
  await f.fire("easing:auto-apply", "click");
  await f.fire("easing:curve", "change", { value: [0.4, 0, 0.6, 1] });
  await f.fire("easing:copy", "click");
  expect(f.node("easing:copy").props["aria-pressed"]).toBe(true);
  await f.refresh({ selection: [1, ["b"], []], curve: [0, 0, 1, 1] });
  await f.fire("easing:paste", "click");
  await f.settle();
  expect(f.actions).toEqual([{ type: "curve", curve: [...from] }]);
  expect(f.node("easing:curve").props.value).toEqual([...from]);
  await f.root.dispose();
});
