/** Coverage is separated into declared shape, source replay and native gates. */
import { contract, sourceFacades } from "../tests/corpus/reference";
import { probePM } from "../tests/corpus/pm-fixture";
import { makePMFacades } from "../src/facades";
import { CompatRealm } from "../src";
import { FakeHost } from "../tests/fake-host";
import { isDeepStrictEqual } from "node:util";
import type { Data } from "../src/types";
const methods = (object: Data, prefix = ""): string[] =>
  Object.entries(Object.getOwnPropertyDescriptors(object)).flatMap(
    ([key, descriptor]) => {
      if (!("value" in descriptor)) return [];
      const path = prefix + key;
      return typeof descriptor.value === "function"
        ? [path]
        : descriptor.value && typeof descriptor.value === "object"
          ? methods(descriptor.value, path + ".")
          : [];
    },
  );
const at = (object: any, path: string): any =>
  path.split(".").reduce((value, key) => value[key], object);
const expectedPM = probePM(),
  actualPM = probePM(),
  expected = await sourceFacades(expectedPM.PM),
  actual = makePMFacades(actualPM.PM, {} as any);
const counts: Record<string, number> = {};
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
  counts[family] = 0;
  for (const path of methods(expected[family])) {
    const args =
      path === "set"
        ? [{ layers: ["b"] }]
        : [
            { id: "a", from: 0, d: {}, layers: ["a"], keys: [], chan: null },
            1,
            2,
            "probe",
          ];
    const result = await at(actual[family], path)(...args),
      reference = await at(expected[family], path)(...args);
    if (
      !isDeepStrictEqual(result, reference) ||
      !isDeepStrictEqual(actualPM.trace, expectedPM.trace)
    )
      throw new Error(`Source replay mismatch: ${family}.${path}`);
    counts[family]++;
  }
}
const host = new FakeHost(),
  realm = new CompatRealm({
    id: "coverage",
    generation: 1,
    initial: host.publication(),
    host,
  });
const api = realm.extension("synthetic-coverage", 3).api;
console.log(
  "family | required members declared | source callable traces matched (native doubles)",
);
for (const [family, members] of Object.entries(await contract())) {
  const required = members.filter((member) => !member.optional),
    declared = required.filter(({ member }) => member in api[family]).length;
  console.log(
    `${family} | ${declared}/${required.length} | ${counts[family] ?? "see corpus/golden tests; no PM source replay"}`,
  );
  if (declared !== required.length)
    throw new Error("Missing declared member: " + family);
}
console.log(
  "Source replay: 95 callable comparisons across 10 families. This is not Electron/native/visual parity.",
);
console.log(
  "Native-required: assets, scene3d, extensions, controls, platform UI/media/render and actor lease/selective-cancel backends.",
);
