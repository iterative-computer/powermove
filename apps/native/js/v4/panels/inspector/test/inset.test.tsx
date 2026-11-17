import { expect, test } from "bun:test";
import { createRoot } from "../../../src";
import { InspectorPanel } from "../view";
import { fixture } from "./fixture";

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
