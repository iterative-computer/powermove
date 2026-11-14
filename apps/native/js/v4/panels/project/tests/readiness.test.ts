import { expect, test } from "bun:test";
import { UnknownReadiness } from "../readiness";
import { asset, projection } from "./fixtures";
import { cardActions, meta, status, accessibleName } from "../presentation";
test("Unknown is neutral until two seconds and never promotes action readiness", () => {
  const clock = new UnknownReadiness();
  const row = asset("a", "Unknown");
  const view = projection([row]);
  clock.observe(view, 0);
  expect(clock.loading(row, 1999)).toBe(false);
  expect(clock.next(view, 1999)).toBe(1);
  expect(clock.loading(row, 2000)).toBe(true);
  expect(clock.next(view, 2000)).toBeNull();
  expect(status("Unknown", (row.data as any).Asset[0])).toBeNull();
  expect(cardActions(row, 1).map((a) => a.label)).toEqual(["Delete Media"]);
  expect(accessibleName(row)).not.toContain("loading");
  const blank = asset("blank", "Unknown");
  if ("Asset" in blank.data) {
    blank.data.Asset[0].format = null;
    blank.data.Asset[0].width = null;
    blank.data.Asset[0].height = null;
    blank.data.Asset[0].duration = null;
    blank.data.Asset[0].bytes = null;
  }
  expect(meta(blank)).toBe("");
  clock.observe(projection([asset("a", "Ready")]), 2200);
  clock.observe(view, 3000);
  expect(clock.loading(row, 3000)).toBe(false);
  const replaced = structuredClone(view);
  replaced.page.stamp.document++;
  clock.observe(replaced, 5000);
  expect(clock.loading(row, 5000)).toBe(false);
});
test("Changed exposes warning detail and locate without inventing readiness", () => {
  const row = asset("a", { Changed: { source: "/synthetic/a.mov" } });
  expect(meta(row)).toBe("Changed · /synthetic/a.mov");
  expect(
    status({ Changed: { source: "x" } }, (row.data as any).Asset[0])?.tone,
  ).toBe("warning");
  expect(cardActions(row, 1).map((a) => a.label)).toEqual([
    "Locate File…",
    "Delete Media",
  ]);
});
test("an HDR video's row says what its light is, after its format", () => {
  const row = asset("a", "Ready");
  if (!("Asset" in row.data)) throw new Error("asset row");
  const plain = meta(row);
  row.data.Asset[1].hdr = "Dolby Vision 8.4";
  const [format, ...rest] = plain.split(" · ");
  expect(meta(row)).toBe([format, "Dolby Vision 8.4", ...rest].join(" · "));
});
