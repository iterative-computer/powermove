import { expect, test } from "bun:test";
import { ViewerPanel, type ViewerActions } from "../panel";
import { fixture, recorder } from "./helpers";

const actions: ViewerActions = { drive() {}, open() {}, command() {} };

test("2.5D shows the navigation switch without Studio preview quality", async () => {
  const state = fixture();
  state.model!.studio = { tier: "fast", look: "camera", hasScene: false };
  const { root } = recorder();
  await root.render(<ViewerPanel state={state} actions={actions} />);
  const ids = root.inspect().nodes.map((node) => node.props.id);
  expect(ids).toContain("viewer.dimension");
  expect(ids).not.toContain("viewer.studio.tier");
  await root.dispose();
});
