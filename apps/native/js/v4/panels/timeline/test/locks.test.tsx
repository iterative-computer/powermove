import { expect, test } from "bun:test";
import { TimelinePanel } from "../panel";
import type { Intent } from "../model";
import { createRoot } from "../../../src/renderer";
import { frame, layer } from "./fixtures";

test("shows a collapsed group's lock badge and offers to unlock a locked layer", async () => {
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  const group = {
    ...layer("group"),
    layerKind: "group",
    containsLockedLayers: true,
  };
  const locked = { ...layer("locked"), locked: true, lockRow: "layer:locked" };
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel
      frame={frame({
        window: { first: 0, total: 2, rows: [group, locked] },
      })}
      send={(intent) => actions.push(intent)}
    />,
  );

  const find = (id: string) =>
    [...root.host.nodes.values()].find((node) => node.props.id === id);
  const badge = find(`timeline:locked-children:${group.id}`);
  expect(badge?.props.label).toBe("Contains locked layers");
  expect(badge?.props.attr).toEqual({ symbol: "lock" });
  const unlock = find(`timeline:lock:${locked.id}`)!;
  expect(unlock.props.label).toBe("Unlock Layer");
  await root.drive(unlock.id, "click");
  expect(actions).toEqual([{ type: "switch", name: "lock", id: locked.id }]);
  await root.dispose();
});
