import { expect, test } from "bun:test";
import { ProjectView, trailingInk } from "../index";
import { asset, folder, projection } from "./fixtures";
import { layout, native, recorder } from "./helpers";
import { token } from "../../../src/components";

for (const selected of [false, true]) {
  test(`missing media uses the same quiet text in grid and list (selected=${selected})`, async () => {
    for (const viewMode of ["grid", "list"] as const) {
      const row = asset("missing", "Offline");
      row.selected = selected;
      const { root } = recorder();
      await root.render(
        <ProjectView
          view={projection([row])}
          viewMode={viewMode}
          native={native}
          layout={layout}
          input={() => {}}
          menu={() => {}}
          page={() => {}}
        />,
      );
      const nodes = root.inspect().nodes;
      const missing = nodes.find((n) => n.text === "Missing");
      expect(missing).toBeDefined();
      expect(
        nodes.find((n) => n.children.includes(missing!.id))!.props.style,
      ).toMatchObject({
        fontSize: token("typography.minimum"),
        color: token(
          trailingInk({ overlay: "Missing", tone: "danger" }, selected),
        ),
      });
      expect(
        nodes.find((n) => n.children.includes(missing!.id))!.props.style,
      ).not.toHaveProperty("background");
      expect(JSON.stringify(nodes)).not.toContain("Media offline");
      expect(JSON.stringify(nodes)).not.toContain("exclamationmark");
      await root.dispose();
    }
  });
}
for (const [sync, label] of [
  ["Syncing", "Syncing…"],
  ["NotFound", "Folder not found"],
  ["Synced", "4 items"],
] as const) {
  test(`folder ${sync} stays quiet in grid and list`, async () => {
    for (const viewMode of ["grid", "list"] as const) {
      const row = folder();
      if ("Folder" in row.data) {
        row.data.Folder.link = { path: "/synthetic/linked" };
        row.data.Folder.sync = sync;
      }
      const { root } = recorder();
      await root.render(
        <ProjectView
          view={projection([row])}
          viewMode={viewMode}
          native={native}
          layout={layout}
          input={() => {}}
          menu={() => {}}
          page={() => {}}
        />,
      );
      const nodes = root.inspect().nodes;
      expect(nodes.some((n) => n.text === label)).toBe(true);
      expect(JSON.stringify(nodes)).not.toContain("exclamationmark");
      expect(
        nodes.some(
          (n) => (n.props.attr as { symbol?: string })?.symbol === "link",
        ),
      ).toBe(false);
      await root.dispose();
    }
  });
}

for (const availability of [
  { Failed: "Synthetic failure" },
  { Cloud: { provider: "Fixture", download: { Failed: "Synthetic failure" } } },
] as const) {
  test(`grid ${JSON.stringify(availability)} status has no severity glyph`, async () => {
    const { root } = recorder();
    await root.render(
      <ProjectView
        view={projection([asset("a", availability)])}
        viewMode="grid"
        native={native}
        layout={layout}
        input={() => {}}
        menu={() => {}}
        page={() => {}}
        transportFailed
      />,
    );
    const nodes = root.inspect().nodes;
    expect(
      nodes.some((n) => n.text === "Project media isn’t updating right now."),
    ).toBe(true);
    expect(JSON.stringify(nodes)).not.toContain("exclamationmark");
    const status = nodes.find(
      (n) =>
        n.props.label ===
        ("Failed" in availability ? "Media unavailable" : "In cloud"),
    );
    expect(status).toBeDefined();
    expect(status!.props.style).not.toHaveProperty("background");
    await root.dispose();
  });
}
