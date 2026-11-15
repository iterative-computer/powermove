import { expect, test } from "bun:test";
import { TimelinePanel } from "../panel";
import type { Intent } from "../model";
import { frame, states } from "./fixtures";
import { createRoot, type SurfaceRoot } from "../../../src/renderer";
function recorder() {
  return {
    root: createRoot(async (c) => ({
      accepted: true,
      revision: c.baseRevision + 1,
    })),
  };
}
function byId(root: SurfaceRoot, id: string) {
  return [...root.host.nodes.values()].find((n) => n.props.id === id)!;
}
async function drive(
  root: SurfaceRoot,
  id: string,
  type = "click",
  detail = {},
) {
  await root.drive(byId(root, id).id, type, detail);
}

test("pill reserves close space and reveals it only for active or hovered tabs", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel
      frame={states["rename-and-tabs"]()}
      send={(a) => actions.push(a)}
    />,
  );
  const close = () => byId(root, "timeline:tab-close:end");
  expect(close().props.label).toBe("Close Endcard");
  expect(close().props["aria-hidden"]).toBe(true);
  expect(byId(root, "timeline:tab-close:launch").props["aria-hidden"]).toBe(
    false,
  );
  expect(byId(root, "timeline:tab-menu:launch")).toBeUndefined();
  expect(close().parent?.props.id).toBe("timeline:tab:end");
  const originalStyle = structuredClone(close().props.style);
  await drive(root, "timeline:tab:end", "pointerenter");
  expect(close().props["aria-hidden"]).toBe(false);
  expect(close().props.style).toEqual(originalStyle);
  await drive(root, "timeline:tab:end", "pointerleave");
  expect(close().props["aria-hidden"]).toBe(true);
  await drive(root, "timeline:tab-close:end");
  expect(actions).toEqual([{ type: "tab", id: "end", close: true }]);
  await root.render(<TimelinePanel frame={frame()} send={() => {}} />);
  expect(byId(root, "timeline:tab-close:launch")).toBeUndefined();
  await root.dispose();
});

test("right-click and Control-click open the composition menu without activation", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel
      frame={states["rename-and-tabs"]()}
      send={(a) => actions.push(a)}
    />,
  );
  await drive(root, "timeline:tab:end", "pointerdown", { button: 2 });
  await drive(root, "timeline:tab:end", "click", { button: 2 });
  await drive(root, "timeline:tab:end", "pointerdown", {
    button: 0,
    ctrlKey: true,
  });
  await drive(root, "timeline:tab:end", "click", { button: 0, ctrlKey: true });
  await drive(root, "timeline:tab-close:end", "pointerdown", {
    button: 0,
    ctrlKey: true,
  });
  await drive(root, "timeline:tab-close:end", "click", {
    button: 0,
    ctrlKey: true,
  });
  expect(actions).toEqual([
    { type: "tabContext", id: "end" },
    { type: "tabContext", id: "end" },
    { type: "tabContext", id: "end" },
  ]);
  await root.dispose();
});

test("tab keyboard activation ignores keys aimed at its nested close button", async () => {
  const { root } = recorder();
  const actions: Intent[] = [];
  await root.render(
    <TimelinePanel
      frame={states["rename-and-tabs"]()}
      send={(a) => actions.push(a)}
    />,
  );
  await drive(root, "timeline:tab:end", "keydown", { key: "Enter" });
  expect(actions).toEqual([{ type: "tab", id: "end", close: false }]);
  actions.length = 0;
  await drive(root, "timeline:tab-close:launch", "keydown", { key: "Enter" });
  expect(actions).toEqual([]);
  await root.dispose();
});
