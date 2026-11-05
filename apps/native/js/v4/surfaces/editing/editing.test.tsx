import { expect, test } from "bun:test";
import { createRoot, type Commit } from "../../src";
import type { SurfaceRoot } from "../../src/renderer";
import { TextAnimators } from "./index";
import {
  copy,
  presets,
  menu,
  propertyMenu,
  dispatch,
  type Publication,
  type Action,
  type ActionBody,
  type MenuItem,
  type EditingServices,
} from "./model";
import {
  ColourPicker,
  notations,
  hueMappings,
  notationString,
  copyString,
  parseNotation,
} from "./colour";
import { fromP3, toOklch, toHsv, toP3 } from "../../src/color-space";

function publication(): Publication {
  return {
    project: "P",
    document: 1,
    revision: 4,
    layer: "T",
    locked: false,
    textSelection: { layer: "T", start: 0, end: 2 },
    animators: [
      {
        id: "a",
        name: "Fade",
        enabled: false,
        animated: true,
        enabledBinding: "enabled/a",
        nameBinding: "name/a",
        selector: [
          {
            id: "start",
            label: "Start",
            kind: "number",
            value: 0,
            unit: "%",
            binding: "start/a",
          },
          {
            id: "order",
            label: "Order",
            kind: "choice",
            value: "random",
            binding: "order/a",
            options: [{ v: "random", label: "Random" }],
          },
        ],
        properties: [
          {
            id: "opacity",
            label: "Opacity",
            kind: "number",
            value: 0,
            unit: "%",
            binding: "opacity/a",
          },
        ],
        propertyCatalogue: [
          { id: "opacity", label: "Opacity" },
          { id: "x", label: "Position X" },
        ],
      },
    ],
  };
}
function record() {
  const actions: Action[] = [];
  const menus: {
    anchor: string;
    items: MenuItem[];
    select: (action: ActionBody) => void;
  }[] = [];
  const commits: Commit[] = [];
  const services: EditingServices = {
    enqueue: (a) => actions.push(a),
    menu: (anchor, items, _stamp, select) =>
      menus.push({ anchor, items, select }),
  };
  const root = createRoot(async (c) => {
    commits.push(structuredClone(c));
    return { accepted: true, revision: c.baseRevision + 1 };
  });
  return { root, services, actions, menus, commits };
}
function byLabel(root: SurfaceRoot, label: string, component?: string) {
  const node = [...root.host.nodes.values()].find(
    (n) =>
      n.props.label === label &&
      (!component || n.props["data-pm-component"] === component),
  );
  if (!node) throw new Error(`Missing ${label}`);
  return node;
}
async function event(root: SurfaceRoot, id: number, type: string, detail = {}) {
  return root.dispatch(
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

test("S034 exact empty state, section, add and style copy", async () => {
  const { root, services } = record();
  const p = publication();
  p.animators = [];
  p.textSelection = undefined;
  await root.render(<TextAnimators publication={p} services={services} />);
  const texts = [...root.host.nodes.values()]
    .map((n) => n.textContent)
    .join(" ");
  expect(texts).toContain("Text Animators");
  expect(texts).toContain(
    "Add an animator to move, fade or style characters over time.",
  );
  expect(byLabel(root, copy.add).props.title).toBe("Add Text Animator");
  expect(byLabel(root, copy.style).props.title).toBe(
    "Select text on the canvas first",
  );
  expect(byLabel(root, copy.style).props.disabled).toBe(true);
});
test("S034 disabled animator keeps native property wells editable and has exact AX names", async () => {
  const { root, services } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  expect(byLabel(root, "Fade, animator 1 of 1").props.role).toBe("group");
  expect(byLabel(root, "Fade enabled", "checkbox").props.value).toBe(false);
  expect(byLabel(root, "Opacity", "number-field").props.disabled).toBe(false);
  expect(byLabel(root, "Shuffle Order").props.title).toBe("Shuffle Order");
  expect(
    (byLabel(root, "Shuffle Order").props.attr as Record<string, string>)[
      "data-pm-symbol"
    ],
  ).toBe("shuffle");
});
test("S034 collapsed animated header removes fields and preserves half diamond state", async () => {
  const { root, services } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  const disclosure = [...root.host.nodes.values()].find(
    (n) => n.props["aria-expanded"] === true,
  )!;
  await event(root, disclosure.id, "click");
  expect(
    [...root.host.nodes.values()].some(
      (n) => n.props["data-pm-component"] === "number-field",
    ),
  ).toBe(false);
  expect(
    [...root.host.nodes.values()].some(
      (n) =>
        (n.props.attr as Record<string, unknown>)?.[
          "data-pm-keyframe-state"
        ] === "animated",
    ),
  ).toBe(true);
});
test("S034 exact menu order, separator, danger and reorder boundaries", () => {
  const p = publication();
  const items = menu(p, "a");
  expect(items.map((m) => m.label)).toEqual([
    "Rename…",
    "Duplicate",
    "Move Up",
    "Move Down",
    "Remove",
  ]);
  expect(items[2].disabled).toBe(true);
  expect(items[3].disabled).toBe(true);
  expect(items[4].danger).toBe(true);
  expect(items[4].separatorBefore).toBe(true);
  p.locked = true;
  expect(menu(p, "a").every((m) => m.disabled)).toBe(true);
  expect(menu(p, "missing")).toEqual([]);
  expect(propertyMenu(p.animators[0]).map((m) => m.label)).toEqual([
    "Position X",
  ]);
});
test("S034 shipped preset ids and Title Case labels remain available", () => {
  expect(presets.map((p) => p[0])).toEqual([
    "fade",
    "rise",
    "rise-words",
    "pop",
    "drop",
    "tumble",
    "blur",
    "tracking",
    "typewriter",
    "wave",
    "pulse",
    "custom-stagger",
    "custom-range",
  ]);
  expect(presets.map((p) => p[1])).toEqual([
    "Fade",
    "Rise",
    "Rise by Word",
    "Pop",
    "Spring Drop",
    "Tumble",
    "Blur In",
    "Tracking In",
    "Typewriter",
    "Wave",
    "Pulse",
    "Stagger",
    "Range Selector",
  ]);
});
test("S034 menus go to the native menu host with a stable anchor", async () => {
  const { root, services, menus } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  await event(root, byLabel(root, copy.add).id, "click");
  expect(menus[0].anchor).toBe("editing/P/1/T/add");
  expect(menus[0].items.map((m) => m.label)).toEqual(presets.map((p) => p[1]));
  await event(root, byLabel(root, copy.addProperty).id, "click");
  expect(menus[1].items.map((m) => m.label)).toEqual(["Position X"]);
});
test("S034 all edits carry project, document, revision and stable object ids", () => {
  const { services, actions } = record();
  const p = publication();
  expect(
    dispatch(p, services, {
      type: "write",
      animator: "a",
      property: "opacity",
      value: 15,
    }),
  ).toBe(true);
  expect(actions).toEqual([
    {
      project: "P",
      document: 1,
      revision: 4,
      layer: "T",
      type: "write",
      animator: "a",
      property: "opacity",
      value: 15,
    },
  ]);
  const before = structuredClone(p);
  p.locked = true;
  expect(dispatch(p, services, { type: "remove", animator: "a" })).toBe(false);
  p.locked = false;
  expect(dispatch(p, services, { type: "remove", animator: "missing" })).toBe(
    false,
  );
  expect(p).toEqual(before);
  expect(actions).toHaveLength(1);
});
test("S034 Style Selected Text validates layer and nonempty range", () => {
  const { services, actions } = record();
  const p = publication();
  expect(dispatch(p, services, { type: "style" })).toBe(true);
  p.textSelection = { layer: "other", start: 0, end: 2 };
  expect(dispatch(p, services, { type: "style" })).toBe(false);
  p.textSelection = { layer: "T", start: 1, end: 1 };
  expect(dispatch(p, services, { type: "style" })).toBe(false);
  expect(actions).toHaveLength(1);
});
test("S034 locked rows refuse field edits, style and preset activation", async () => {
  const { root, services, actions, menus } = record();
  const p = publication();
  p.locked = true;
  await root.render(<TextAnimators publication={p} services={services} />);
  expect(byLabel(root, "Opacity", "number-field").props.disabled).toBe(true);
  await event(root, byLabel(root, copy.style).id, "click");
  await event(root, byLabel(root, copy.add).id, "click");
  expect(actions).toEqual([]);
  expect(menus).toEqual([]);
});
test("S034 commits use only six atomic ops and validate with the real native UI kit", async () => {
  const { root, services, commits } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  const allowed = [
    "create",
    "setProps",
    "setText",
    "insertBefore",
    "remove",
    "setListeners",
  ];
  expect(
    commits.flatMap((c) => c.ops).every((o) => allowed.includes(o.op)),
  ).toBe(true);
  const wire = process.env.PM_UI_KIT_WIRE;
  expect(wire).toBeTruthy();
  const child = Bun.spawnSync([wire!], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(child.exitCode).toBe(0);
  const result = JSON.parse(child.stdout.toString().trim());
  expect(result.accepted).toBe(true);
  expect(result.diagnostics).toEqual([]);
  expect(result.wireDiagnostics).toEqual([]);
});
test("S035 every notation and hue mapping has exact spec copy", () => {
  expect(notations).toEqual(["Hex", "RGB", "HSL", "OKLCH", "OKLab", "P3"]);
  expect(hueMappings).toEqual([
    "Find Closest Color",
    "Maintain Chroma",
    "Maintain Lightness",
  ]);
});
test("S035 Copy as copies the current notation string verbatim", () => {
  for (const notation of notations) {
    const c = { r: 0.2, g: 0.4, b: 0.6, a: 0.5 };
    expect(copyString(c, notation)).toBe(notationString(c, notation));
  }
});
test("S035 all six notation fields parse and OKLab preserves alpha", () => {
  const examples = [
    "#336699",
    "51 102 153",
    "210° 50% 40%",
    "50% 0.1 210°",
    "50% -0.1 0.1 / 25%",
    "0.2 0.4 0.6",
  ];
  notations.forEach((n, i) => {
    const c = parseNotation(examples[i], n);
    expect(c).not.toBeNull();
    expect(Object.values(c!).every(Number.isFinite)).toBe(true);
  });
  expect(parseNotation(examples[4], "OKLab")!.a).toBe(0.25);
  expect(parseNotation("bad", "OKLab")).toBeNull();
  expect(parseNotation("NaN 0 0", "RGB")).toBeNull();
});
test("S035 gamut warning and restore preserve the P3 source colour", () => {
  const original = fromP3({ r: 1, g: 0, b: 0, a: 0.8 });
  const p = new ColourPicker(original);
  expect(p.warning("sRGB")).toBe(
    "Outside sRGB; saved as the nearest sRGB colour",
  );
  expect(p.warning("Display P3")).toBeUndefined();
  p.notation = "Hex";
  expect(p.draft("#112233")).toBe(true);
  p.restore();
  expect(p.colour).toEqual(original);
  expect(p.previous).toEqual(original);
});
test("S035 hue mapping keeps lightness or chroma as authored and ignores invalid input", () => {
  for (const mapping of hueMappings) {
    const p = new ColourPicker({ r: 0.4, g: 0.3, b: 0.5, a: 0.8 });
    const before = toOklch(p.colour);
    p.mapping = mapping;
    expect(p.changeHue(210)).toBe(true);
    expect(p.colour.a).toBe(0.8);
    const after = toOklch(p.colour);
    if (mapping === "Maintain Lightness")
      expect(after.l).toBeCloseTo(before.l, 5);
    if (mapping === "Maintain Chroma") expect(after.c).toBeCloseTo(before.c, 5);
    const current = { ...p.colour };
    expect(p.changeHue(NaN)).toBe(false);
    expect(p.colour).toEqual(current);
  }
});

test("S034 native menu Rename enters an in-place field and Return submits the exact draft", async () => {
  const { root, services, menus, actions } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  const button = [...root.host.nodes.values()].find((n) =>
    String(n.props.id).endsWith("/a/menu"),
  )!;
  await event(root, button.id, "click");
  menus[0].select(menus[0].items[0].action);
  await root.flush();
  const field = byLabel(root, "Fade", "text-field");
  await event(root, field.id, "input", { value: "Authored 🎬" });
  await event(root, field.id, "keydown", { key: "Enter" });
  expect(actions[0]).toEqual({
    project: "P",
    document: 1,
    revision: 4,
    layer: "T",
    type: "rename",
    animator: "a",
    name: "Authored 🎬",
  });
});
test("S034 Escape cancels the rename draft and keeps editor shortcuts local", async () => {
  const { root, services, menus, actions } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  const button = [...root.host.nodes.values()].find((n) =>
    String(n.props.id).endsWith("/a/menu"),
  )!;
  await event(root, button.id, "click");
  menus[0].select(menus[0].items[0].action);
  await root.flush();
  const field = byLabel(root, "Fade", "text-field");
  expect(field.props.preventDefault).toEqual({ keydown: ["Enter", "Escape"] });
  await event(root, field.id, "input", { value: "Discarded draft" });
  await event(root, field.id, "keydown", { key: "Escape" });
  expect(actions).toEqual([]);
  expect(
    [...root.host.nodes.values()].some(
      (n) => n.props["data-pm-component"] === "text-field",
    ),
  ).toBe(false);
});
test("S034 hover reveals a labelled property removal button", async () => {
  const { root, services, actions } = record();
  await root.render(
    <TextAnimators publication={publication()} services={services} />,
  );
  const hovered = [...root.host.nodes.values()].find(
    (n) => n.handlers.has("pointermove:false") && n.handlers.has("focus:false"),
  )!;
  await event(root, hovered.id, "pointermove");
  const remove = byLabel(root, "Remove Opacity");
  expect((remove.props.attr as Record<string, string>)["data-pm-symbol"]).toBe(
    "minus.circle",
  );
  await event(root, remove.id, "click");
  expect(actions[0].type).toBe("remove-property");
});

test("S035 switching picker space changes the plane without rewriting the authored colour", () => {
  const original = fromP3({ r: 0.9, g: 0.2, b: 0.1, a: 0.7 });
  const picker = new ColourPicker(original);
  picker.space = "Display P3";
  expect(picker.colour).toEqual(original);
  const p3Hue = toHsv(toP3(original)).h;
  picker.mapping = "Maintain Lightness";
  picker.changeHue(p3Hue);
  expect(picker.colour.r).toBeCloseTo(original.r, 5);
  expect(picker.colour.g).toBeCloseTo(original.g, 5);
  picker.restore();
  picker.space = "sRGB";
  expect(picker.colour).toEqual(original);
});
