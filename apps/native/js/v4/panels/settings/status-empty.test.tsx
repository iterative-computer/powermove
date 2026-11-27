import { expect, test } from "bun:test";
import { createRoot } from "../../src/index";
import type { Row } from "./catalogue";
import { bindings, fixture } from "./fixtures";
import { SettingsPanel } from "./index";

// profile-hygiene: a status row without a value emitted an empty trailing
// text node. Under Dark Aqua the offscreen form gave that empty field the
// row's width and crushed the title to nothing (invisible row).
test("a status row without a value emits no empty trailing node", async () => {
  const rows: Row[] = [
    {
      id: "fixture.status",
      section: "general",
      group: "Fixture",
      label: "Status Without Value",
      detail: "The title stays visible in dark mode.",
      control: "status",
    },
  ];
  const { model } = fixture({ rows });
  model.show("general");
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  await root.render(<SettingsPanel model={model} bindings={bindings} />);
  await root.flush();
  const nodes = root.inspect().nodes;
  const row = nodes.find((node) => node.props.id === "settings.row.fixture.status");
  expect(row).toBeDefined();
  const line = row!.children
    .map((id) => nodes.find((node) => node.id === id)!)
    .find((node) => node.props.metadata?.settingsLayout === "line");
  expect(line).toBeDefined();
  const emptyText = line!.children
    .map((id) => nodes.find((node) => node.id === id)!)
    .flatMap((node) => [node, ...node.children.map((cid) => nodes.find((n) => n.id === cid)!)])
    .filter((node) => node.type === "text" && (node.text ?? "") === "");
  expect(emptyText).toEqual([]);
  root.dispose();
  model.dispose();
});
