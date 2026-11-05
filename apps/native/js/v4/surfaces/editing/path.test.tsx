import { expect, test } from "bun:test";
import { createRoot } from "../../src";
import { PathControls, type PathControlsPlan } from "./path";
const plan: PathControlsPlan = {
  pathId: "authored-path",
  vertices: [
    {
      id: "a",
      label: "Point 1 of 2, 1.25, -3 pixels",
      selected: true,
      hovered: true,
      position: { x: 10.25, y: -3 },
      incoming: { x: 2, y: -3 },
      outgoing: { x: 16, y: -2 },
      showTangents: true,
    },
    {
      id: "b",
      label: "Point 2 of 2, 30, 12 pixels",
      selected: false,
      hovered: false,
      position: { x: 30, y: 12 },
      incoming: { x: 24, y: 12 },
      outgoing: { x: 36, y: 12 },
      showTangents: false,
    },
  ],
};
test("path controls use kit gizmos at exact evaluated coordinates and only eligible tangents", async () => {
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(<PathControls plan={plan} input={() => {}} />);
  const gizmos = [...root.host.nodes.values()].filter(
    (n) => n.props["data-pm-component"] === "viewer-gizmo",
  );
  expect(gizmos).toHaveLength(4);
  expect(
    gizmos.map((n) => (n.props.metadata as { pmKit: unknown }).pmKit),
  ).toEqual([
    {
      kind: "path-vertex",
      selected: true,
      hovered: true,
      position: plan.vertices[0].position,
    },
    { kind: "tangent", selected: false, position: plan.vertices[0].incoming },
    { kind: "tangent", selected: false, position: plan.vertices[0].outgoing },
    {
      kind: "path-vertex",
      selected: false,
      hovered: false,
      position: plan.vertices[1].position,
    },
  ]);
  expect(gizmos[0].props.label).toBe(plan.vertices[0].label);
  expect(gizmos[0].props.style).toEqual({
    minWidth: { token: "metrics.hit" },
    minHeight: { token: "metrics.hit" },
  });
  await root.render(
    <PathControls plan={{ ...plan, vertices: [] }} input={() => {}} />,
  );
  expect(
    [...root.host.nodes.values()].filter(
      (n) => n.props["data-pm-component"] === "viewer-gizmo",
    ),
  ).toHaveLength(0);
});
test("path input returns stable path, vertex and handle IDs", async () => {
  const actions: unknown[] = [];
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(
    <PathControls
      plan={plan}
      input={(path, vertex, handle) => actions.push([path, vertex, handle])}
    />,
  );
  const node = [...root.host.nodes.values()].find(
    (n) => n.props.id === "path/authored-path/vertex/a/outgoing",
  )!;
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: node.id,
      deliveries: [{ id: node.id, capture: false }],
      preventDefault: false,
    },
    "pointerdown",
    {},
  );
  expect(actions).toEqual([["authored-path", "a", "outgoing"]]);
});
