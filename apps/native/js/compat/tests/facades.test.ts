import { expect, test } from "bun:test";
import { makePMFacades } from "../src/facades";
import { CompatRealm } from "../src";
import { FakeHost } from "./fake-host";
import { sourceFacades, contract } from "./corpus/reference";
import { probePM } from "./corpus/pm-fixture";
import type { Data } from "../src/types";
const paths = (value: Data, prefix = ""): string[] =>
  Object.entries(Object.getOwnPropertyDescriptors(value)).flatMap(
    ([name, d]) => {
      if (!("value" in d)) return [];
      const key = prefix + name;
      return typeof d.value === "function"
        ? [key]
        : d.value && typeof d.value === "object"
          ? paths(d.value, key + ".")
          : [];
    },
  );
const get = (object: Data, path: string): any =>
  path.split(".").reduce((v, key) => v[key], object);

test("PM facade source replay: every callable in ten PM-backed families preserves trace, arguments and results", async () => {
  const left = probePM(),
    right = probePM();
  const expected = await sourceFacades(left.PM),
    actual = makePMFacades(right.PM, {} as any);
  let compared = 0;
  const args = { id: "a", from: 0, d: {}, layers: ["a"], keys: [], chan: null };
  for (const family of [
    "anim",
    "model",
    "selection",
    "groups",
    "transport",
    "media",
    "render",
    "uiState",
    "util",
    "ease",
  ]) {
    for (const path of paths(expected[family])) {
      const arguments_ =
        path === "set" ? [{ layers: ["b"] }] : [args, 1, 2, "probe"];
      const before = left.trace.length;
      const value = get(actual[family], path)(...arguments_);
      const reference = get(expected[family], path)(...arguments_);
      expect(await value, family + "." + path).toEqual(await reference);
      if (reference === left.marker) expect(value).toBe(right.marker);
      expect(right.trace.slice(before), family + "." + path).toEqual(
        left.trace.slice(before),
      );
      expect(right.PM.sel).toEqual(left.PM.sel);
      compared++;
    }
  }
  expect(compared).toBe(95);
});
test.each([1, 2, 3] as const)(
  "v%d synthetic API contract: every required top-level family member is declared",
  async (version) => {
    const host = new FakeHost(),
      realm = new CompatRealm({
        id: "contract",
        generation: 1,
        initial: host.publication(),
        host,
      });
    const api = realm.extension("synthetic-contract", version).api;
    for (const [family, members] of Object.entries(await contract())) {
      expect(api[family], family).toBeDefined();
      for (const { member, optional } of members)
        if (!optional)
          expect(member in api[family], `${family}.${member}`).toBe(true);
    }
    expect(api.apiVersion).toBe(version);
  },
);
test("native backend accessors are lazy, live and preserved without copying service objects", () => {
  const host = new FakeHost();
  let reads = 0;
  const value = {};
  const realm = new CompatRealm({
    id: "lazy",
    generation: 1,
    initial: host.publication(),
    host,
    capabilities: {
      transport: {
        get perf() {
          reads++;
          return value;
        },
      },
    },
  });
  const api = realm.extension("lazy").api;
  expect(reads).toBe(0);
  expect(api.transport.perf).toBe(value);
  expect(reads).toBe(1);
});
test("missing native facilities fail explicitly at invocation", () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "missing",
      generation: 1,
      initial: host.publication(),
      host,
    });
  const api = realm.extension("missing").api;
  for (const invoke of [
    () => api.anim.worldMatrix({}),
    () => api.assets.pick(),
    () => api.media.fonts.ensure("Synthetic"),
    () => api.scene3d.createScene(),
    () => api.storage.set("synthetic", 1),
    () => api.panels.open("panel"),
    () => api.ui.controls.NumField(),
  ])
    expect(invoke).toThrow("not attached");
});
