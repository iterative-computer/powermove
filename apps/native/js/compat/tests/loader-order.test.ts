import { test, expect } from "bun:test";
import { planLoad } from "../src/loader";

const record = (id: string, dependsOn: string[] = []) => ({
  id,
  enabled: true,
  health: { state: "ok" },
  manifest: { dependsOn },
});

// The host asks for the extensions a project draws with first (PM-384): the
// realm activates them in that order, each after what it depends on, never
// re-sorted by id.
test("activation keeps the host's order, dependencies first", () => {
  const { order } = planLoad([
    record("progressive-blur"),
    record("deep-glow", ["glow-core"]),
    record("anchor-point"),
    record("glow-core"),
  ]);
  expect(order).toEqual([
    "progressive-blur",
    "glow-core",
    "deep-glow",
    "anchor-point",
  ]);
});

test("a cycle, and what depends on it, never activates", () => {
  const { order, skipped } = planLoad([
    record("uses-cycle", ["a"]),
    record("a", ["b"]),
    record("b", ["a"]),
    record("free"),
  ]);
  expect(order).toEqual(["free"]);
  expect(skipped.map((s) => s.id).sort()).toEqual(["a", "b", "uses-cycle"]);
});
