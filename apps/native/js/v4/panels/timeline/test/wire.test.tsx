import { describe, expect, test } from "bun:test";
import { TimelinePanel } from "../panel";
import { checkFrame } from "../model";
import type { Intent } from "../model";
import { states, frame } from "./fixtures";
import { recorder, nativeValidators } from "./helpers";
import type { SurfaceRoot } from "../../../src/renderer";
import type { Commit } from "../../../src/protocol";

function replay(commits: Commit[], flags: string[] = []) {
  const result = Bun.spawnSync([nativeValidators().surface, ...flags], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  expect(result.exitCode).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
function tree(root: SurfaceRoot) {
  const visit = (node: typeof root.host.root): unknown => ({
    type: node.type,
    props: node.props,
    ...(node.textContent && !node.children.length
      ? { text: node.textContent }
      : {}),
    ...(node.children.length ? { children: node.children.map(visit) } : {}),
  });
  return visit(root.host.root);
}
function byId(root: SurfaceRoot, id: string) {
  return [...root.host.nodes.values()].find((n) => n.props.id === id)!;
}
function focusIds(root: SurfaceRoot, focus: number[]) {
  return focus.map(
    (id) =>
      root.host.nodes.get(id)!.props.id ?? root.host.nodes.get(id)!.props.label,
  );
}
async function drive(
  root: SurfaceRoot,
  id: string,
  type = "click",
  detail = {},
) {
  await root.drive(byId(root, id).id, type, detail);
}

describe("timeline: accepted six-op trees (provisional placeholders, not visual acceptance)", () => {
  for (const [name, make] of Object.entries(states))
    test(name, async () => {
      const { root, commits } = recorder();
      const f = checkFrame(make(), { first: 0, visible: 10 });
      await root.render(<TimelinePanel frame={f} send={() => {}} />);
      for (const result of replay(commits)) {
        expect(result.accepted).toBe(true);
        expect(result.diagnostics).toEqual([]);
        expect(result.wireDiagnostics).toEqual([]);
        expect(result.accessibilityNodes).toBe(
          Object.keys(result.nodes).length,
        );
      }
      expect(tree(root)).toMatchSnapshot();
      await root.dispose();
    });
});

test("one track-area focus stop, non-focusable rows, conditional graph menu and tabs", async () => {
  const { root, commits } = recorder();
  await root.render(<TimelinePanel frame={frame()} send={() => {}} />);
  // v1's order: the tab row (tabs, then Layers / Tracks), then the head over
  // the layer column (play, time, graph switch), then the tracks.
  expect(focusIds(root, replay(commits).at(-1).focusOrder)).toEqual([
    "timeline:tab:launch",
    "P0-0:Layers",
    "timeline:play",
    "timeline:time",
    "timeline:graph",
    "timeline:tracks",
  ]);
  await root.render(
    <TimelinePanel frame={states["graph-empty"]()} send={() => {}} />,
  );
  expect(focusIds(root, replay(commits).at(-1).focusOrder)).toContain(
    "timeline:graph-options",
  );
  expect(byId(root, "timeline:tab-close:launch")).toBeUndefined();
  expect(byId(root, "timeline:tracks").props.label).toBe("Layers");
  expect(byId(root, "timeline:row:layer:headline").props["aria-selected"]).toBe(
    false,
  );
  await root.dispose();
});

test("header actions preserve model enums, snap intent and composition identity", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel
      frame={states["rename-and-tabs"]()}
      send={(a) => actions.push(a)}
    />,
  );
  for (const id of [
    "timeline:play",
    "timeline:time",
    "timeline:tab:end",
    "timeline:tab-close:launch",
  ])
    await drive(root, id);
  const tracks = [...root.host.nodes.values()].find(
    (n) => n.props.role === "radio" && n.props.label === "Tracks",
  )!;
  await root.drive(tracks.id, "click");
  expect(actions).toEqual([
    { type: "play" },
    { type: "timeEdit" },
    { type: "tab", id: "end", close: false },
    { type: "tab", id: "launch", close: true },
    { type: "mode", mode: "Tracks" },
  ]);
  await root.dispose();
});

test("v1 head: one graph switch, its options while shown, snapping only in Tracks", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  const send = (a: Intent) => actions.push(a);
  // The switch returns to the graph view last shown: start from Graph.
  await root.render(
    <TimelinePanel frame={{ ...frame(), view: "Graph" }} send={send} />,
  );
  await root.render(<TimelinePanel frame={frame()} send={send} />);
  // Layers mode, dope sheet: no options, no snapping (v1 hides the magnet).
  expect(byId(root, "timeline:graph-options")).toBeUndefined();
  expect(byId(root, "timeline:snap")).toBeUndefined();
  expect(byId(root, "timeline:view")).toBeUndefined();
  await drive(root, "timeline:graph");
  // Shown: the switch is lit and turns it off; the options appear.
  await root.render(
    <TimelinePanel frame={{ ...frame(), view: "Combined" }} send={send} />,
  );
  expect(byId(root, "timeline:graph-options")).toBeDefined();
  await drive(root, "timeline:graph");
  // Off again, it comes back to the view last shown (layer bars kept).
  await root.render(
    <TimelinePanel frame={{ ...frame(), view: "DopeSheet" }} send={send} />,
  );
  await drive(root, "timeline:graph");
  await root.render(
    <TimelinePanel frame={{ ...frame(), mode: "Tracks" }} send={send} />,
  );
  await drive(root, "timeline:snap");
  expect(actions).toEqual([
    { type: "view", view: "Graph" },
    { type: "view", view: "DopeSheet" },
    { type: "view", view: "Combined" },
    { type: "snap" },
  ]);
  await root.dispose();
});

test("modifiers reach native selection, child switches never also select the row", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel frame={states.hover()} send={(a) => actions.push(a)} />,
  );
  await drive(root, "timeline:row:layer:headline", "click", {
    shiftKey: true,
    metaKey: true,
  });
  await drive(root, "timeline:solo:layer:headline");
  expect(actions).toEqual([
    {
      type: "selectRow",
      id: "layer:headline",
      modifiers: {
        shiftKey: true,
        metaKey: true,
        altKey: false,
        ctrlKey: false,
      },
    },
    { type: "switch", id: "layer:headline", name: "solo" },
  ]);
  await root.dispose();
});

test("locked key/value controls are disabled, layer selection remains available", async () => {
  const { root, commits } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel frame={states.disabled()} send={(a) => actions.push(a)} />,
  );
  const diamond = byId(root, "timeline:diamond:property:opacity");
  const value = byId(root, "timeline:value:property:opacity");
  expect(diamond.props.disabled).toBe(true);
  expect(value.props.disabled).toBe(true);
  await root.drive(diamond.id, "click");
  expect(actions).toEqual([]);
  await drive(root, "timeline:row:layer:headline");
  expect(actions[0].type).toBe("selectRow");
  expect(replay(commits).at(-1).focusOrder).not.toContain(diamond.id);
  await root.dispose();
});

test("property navigation, key toggle and linked scale keep original native IDs", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel
      frame={states["scale-compound"]()}
      send={(a) => actions.push(a)}
    />,
  );
  for (const name of ["previous", "diamond", "next", "scale-link"])
    await drive(root, `timeline:${name}:property:scale`);
  expect(actions).toEqual([
    {
      type: "adjacentKey",
      layer: "headline",
      path: "scale",
      direction: "previous",
    },
    { type: "keyToggle", layer: "headline", path: "scale" },
    {
      type: "adjacentKey",
      layer: "headline",
      path: "scale",
      direction: "next",
    },
    { type: "scaleLink", layer: "headline" },
  ]);
  await root.dispose();
});

test("native key admission owns shortcuts; panel forwards modifiers/repeat, adding no chords", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel frame={frame()} send={(a) => actions.push(a)} />,
  );
  const keys = [
    " ",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "j",
    "k",
    "i",
    "o",
    "[",
    "]",
    "b",
    "n",
    "p",
    "s",
    "r",
    "t",
    "a",
    "u",
    "m",
    "f",
    "e",
    "l",
    "`",
    "F3",
    "F9",
    "ArrowUp",
    "ArrowDown",
    "Enter",
    "Escape",
    "Shift",
    "Tab",
  ];
  for (const key of keys)
    await drive(root, "timeline:tracks", "keydown", {
      key,
      shiftKey: true,
      metaKey: true,
      repeat: true,
    });
  expect(actions).toEqual(
    keys.map((key) => ({
      type: "keyDown",
      event: { key, shiftKey: true, metaKey: true, repeat: true },
    })),
  );
  await root.dispose();
});

test("60k projection mounts only the bounded native window and exposes full count as pending AX metadata", async () => {
  const { root, commits } = recorder();
  await root.render(
    <TimelinePanel frame={states["60k-rows"]()} send={() => {}} />,
  );
  expect(
    [...root.host.nodes.values()].filter(
      (n) => n.props["data-pm-component"] === "tree-item",
    ),
  ).toHaveLength(30);
  expect(byId(root, "timeline:tracks").props.metadata).toMatchObject({
    totalRows: 60000,
    firstRow: 29990,
    indexDigits: 5,
  });
  expect(root.host.nodes.size).toBeLessThan(400);
  expect(replay(commits).at(-1).accessibilityNodes).toBe(root.host.nodes.size);
  await root.dispose();
});

test("playback changes time/transport only, without row operations or new row nodes", async () => {
  const { root, commits } = recorder();
  const send = () => {};
  await root.render(<TimelinePanel frame={frame()} send={send} />);
  const row = byId(root, "timeline:row:layer:headline");
  const descendants = new Set<number>();
  const visit = (node: typeof row) => {
    descendants.add(node.id);
    node.children.forEach(visit);
  };
  visit(row);
  const prior = commits.length;
  await root.render(<TimelinePanel frame={states.playing()} send={send} />);
  expect(byId(root, "timeline:row:layer:headline").id).toBe(row.id);
  expect(
    commits
      .slice(prior)
      .flatMap((c) => c.ops)
      .filter((op) => "id" in op && descendants.has(op.id)),
  ).toEqual([]);
  await root.dispose();
});

test("actual AccessKit tree exposes panel/outline labels, selection, disabled values and spoken time", async () => {
  const { root, commits } = recorder();
  const f = states.disabled();
  f.window.rows[0].selected = true;
  await root.render(<TimelinePanel frame={f} send={() => {}} />);
  const tree = replay(commits).at(-1).accessibility;
  const ax = (id: string) =>
    tree.find((node: { id: number }) => node.id === byId(root, id).id);
  expect(ax("timeline")).toMatchObject({ role: "Group", label: "Timeline" });
  expect(ax("timeline:tracks")).toMatchObject({
    role: "Tree",
    label: "Layers",
  });
  expect(ax("timeline:row:layer:headline")).toMatchObject({
    role: "TreeItem",
    selected: true,
    label: "Headline, text layer, 0:00 to 0:10",
  });
  expect(ax("timeline:value:property:opacity")).toMatchObject({
    role: "NumberInput",
    disabled: true,
    value: "100 percent",
  });
  expect(ax("timeline:time")).toMatchObject({
    label: "Current time",
    value: "0 hours 0 minutes 4 seconds 15 frames",
  });
  expect(ax("timeline:render-bar")).toMatchObject({
    role: "Label",
    label: "Rendered 0:00 to 0:12, rendering 0:12 to 0:14",
  });
  await root.dispose();
});

test("all sixteen scheme/accessibility token combinations accept the panel without literal visuals", async () => {
  const { root, commits } = recorder();
  await root.render(<TimelinePanel frame={states.hover()} send={() => {}} />);
  for (let mask = 0; mask < 16; mask++) {
    const flags = [
      "--light",
      "--contrast",
      "--transparency",
      "--motion",
    ].filter((_, index) => mask & (1 << index));
    for (const result of replay(commits, flags)) {
      expect(result.accepted).toBe(true);
      expect(result.diagnostics).toEqual([]);
    }
  }
  await root.dispose();
});
