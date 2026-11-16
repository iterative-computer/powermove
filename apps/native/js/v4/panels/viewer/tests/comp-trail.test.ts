import { expect, test } from "bun:test";
import { compTrail } from "../comp-trail";
const parents = (id: string) =>
  (({ Logo: ["Main"], Spinner: ["Logo"] }) as Record<string, string[]>)[id] ??
  [];
test("rebuilds nested ancestry", () => {
  expect(compTrail(null, "Spinner", parents)).toEqual({
    ids: ["Main", "Logo", "Spinner"],
    index: 2,
  });
});
test("retains descendants when navigating back", () => {
  const trail = compTrail(null, "Spinner", parents);
  expect(compTrail(trail, "Main", parents)).toEqual({
    ids: trail.ids,
    index: 0,
  });
});
test("trims deleted relationships", () => {
  const trail = compTrail(null, "Spinner", parents);
  expect(compTrail(trail, "Main", () => [])).toEqual({
    ids: ["Main"],
    index: 0,
  });
});
test("bounds cycles and clears empty navigation", () => {
  expect(
    compTrail(null, "a", (id) => (id === "a" ? ["b"] : ["a"])).ids,
  ).toEqual(["b", "a"]);
  expect(compTrail(null, null, parents)).toEqual({ ids: [], index: -1 });
});
