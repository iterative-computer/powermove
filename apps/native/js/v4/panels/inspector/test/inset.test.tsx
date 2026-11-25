import { expect, test } from "bun:test";
import { createRoot } from "../../../src";
import { InspectorPanel } from "../view";
import { binding, fixture, snapshot } from "./fixture";

test("Properties retains the panel inset after its last row", async () => {
  const f = fixture();
  await f.session.start();
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(<InspectorPanel session={f.session} />);
  const content = [...root.host.nodes.values()].find(
    (n) => n.props.id === "inspector:content",
  )!;
  expect(content.props.style).toMatchObject({
    paddingBottom: { token: "metrics.inset" },
  });
  await root.dispose();
});

test("the Text row can grow and centers its controls with compact insets", async () => {
  const model = snapshot();
  model.sections[0].rows.push({
    id: "text",
    label: "Text",
    control: {
      kind: "text",
      multiline: true,
      value: "0:05",
      binding: binding("text"),
    },
  });
  const f = fixture(model);
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  try {
    await f.session.start();
    await root.render(<InspectorPanel session={f.session} />);
    const textRow = [...root.host.nodes.values()].find(
      (node) =>
        node.props["data-pm-component"] === "inspector-row" &&
        node.props.label === "Text",
    );
    expect(textRow?.props.style).toMatchObject({
      height: null,
      minHeight: { token: "metrics.row" },
      alignItems: "center",
      paddingTop: { token: "metrics.pill_inset" },
      paddingBottom: { token: "metrics.pill_inset" },
    });
  } finally {
    await root.dispose();
    await f.session.dispose();
  }
});
