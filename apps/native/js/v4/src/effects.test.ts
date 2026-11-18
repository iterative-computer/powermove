import { test, expect } from "bun:test";
import { effectsAPI } from "./effects";
const definition = {
  id: "probe.fx",
  label: "Probe",
  group: "Test",
  params: [{ k: "amount", label: "Amount", def: 1, min: 0, max: 2 }],
  frag: "o = src(v_uv) * u_amount;",
  passes: 2,
  keepOrig: true,
  backdrop: true,
};
test("v4 effects preserve the legacy shader and flags and dispose", () => {
  let published: any[] = [];
  const api = effectsAPI((values) => {
    published = values;
  });
  const dispose = api.register(definition);
  expect(published[0].snapshot.glsl.source).toContain(
    "uniform float u_amount;",
  );
  expect(published[0].snapshot.glsl.source).toContain(
    "uniform sampler2D u_orig;",
  );
  expect(published[0].snapshot.flags).toMatchObject({
    passes: 2,
    keepOrig: true,
    backdrop: true,
  });
  expect(api.get("probe.fx")).toEqual(definition);
  dispose();
  expect(published).toEqual([]);
});
test("invalid definitions cannot displace a working provider", () => {
  const api = effectsAPI(() => {});
  api.register(definition);
  expect(() => api.register({ ...definition, passes: 9 })).toThrow();
  expect(api.get("probe.fx")).toEqual(definition);
});
test("disposing an override restores the preceding registration", () => {
  const api = effectsAPI(() => {});
  const first = api.register(definition);
  const last = api.register({ ...definition, label: "Override" });
  expect(api.get("probe.fx")?.label).toBe("Override");
  last();
  expect(api.get("probe.fx")?.label).toBe("Probe");
  first();
  first();
  last();
  expect(api.list()).toEqual([]);
});

test("provider tokens survive unrelated additions and reveal shadowed registrations", () => {
  let published: any[] = [];
  const api = effectsAPI((values) => {
    published = values;
  });
  const first = api.register(definition);
  const token = published[0].registration;
  api.register({ ...definition, id: "other.fx" });
  expect(published.map((p) => p.registration)).toEqual([token, token + 1]);
  const top = api.register({ ...definition, label: "Override" });
  expect(published.map((p) => p.registration)).toEqual([
    token,
    token + 1,
    token + 2,
  ]);
  expect(published[0].definition.label).toBe("Probe");
  expect(api.get("probe.fx")?.label).toBe("Override");
  top();
  expect(published.map((p) => p.registration)).toEqual([token, token + 1]);
  first();
  expect(published.map((p) => p.registration)).toEqual([token + 1]);
});
