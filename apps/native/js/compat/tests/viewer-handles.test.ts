import { expect, test } from "bun:test";
import { validateViewerHandles } from "../src/viewer-handles";
const def = (viewerHandles: unknown) => ({
  params: [
    { k: "x" },
    { k: "y" },
    { k: "angle" },
    { k: "radius" },
    { k: "tint", type: "color" },
  ],
  viewerHandles,
});
test("viewer declarations cover each control kind", () => {
  const p = { x: "x", y: "y" };
  expect(() =>
    validateViewerHandles(
      def([
        { kind: "point", point: p },
        { kind: "line", start: p, end: p },
        { kind: "radius", center: p, radius: "radius" },
        { kind: "angle", center: p, angle: "angle" },
        { kind: "rect", start: p, end: p },
      ]),
    ),
  ).not.toThrow();
});
test("viewer declarations reject unknown keys, fields and kinds", () => {
  for (const h of [
    { kind: "point", point: { x: "missing", y: "y" } },
    { kind: "point", point: { x: "tint", y: "y" } },
    { kind: "point", point: { x: "x", y: "y" }, unknown: 1 },
    { kind: "ellipse" },
  ]) {
    expect(() => validateViewerHandles(def([h]))).toThrow();
  }
});

import {
  gradientDeclarations,
  validateGradientStops,
} from "../src/viewer-handles";
import { toLegacyFx } from "../src/frozen/host/glsl";
import { definitionSnapshots } from "../src/definitions";
const stops = [
  { position: 0, color: [0, 0, 0, 1] },
  { position: 1, color: [1, 1, 1, 1] },
];
test("gradient stops validate bounds, cardinality and RGBA", () => {
  expect(() => validateGradientStops(stops)).not.toThrow();
  for (const value of [
    [],
    [stops[0]],
    Array(17).fill(stops[0]),
    [{ position: NaN, color: [0, 0, 0, 1] }, stops[1]],
    [{ position: 0, color: [0, 0, 2, 1] }, stops[1]],
  ])
    expect(() => validateGradientStops(value)).toThrow();
});
test("agent shader effects receive named stop uniforms and sampler", () => {
  const definition = {
    id: "agentRamp",
    label: "Agent ramp",
    group: "Test",
    params: [{ k: "ramp", label: "Ramp", type: "gradientStops", def: stops }],
    frag: "o=pm_ramp(v_uv.x);",
  };
  const shader = toLegacyFx(
    definition,
    "#version 300 es\nprecision highp float;\nout vec4 o;\nin vec2 v_uv;",
  ).frag;
  expect(shader).toContain("uniform vec4 u_rampC15;");
  expect(shader).toContain("vec4 pm_ramp(float t)");
  expect(shader).toContain("u_rampCount>15.5");
  expect(shader).not.toContain("uniform float u_ramp;");
  expect(gradientDeclarations("ramp")).toContain("uniform float u_rampP15");
  const registry = {
    entries: () => [{ id: "agentRamp", ownerId: "agent", item: definition }],
  };
  const empty = { entries: () => [] };
  const snapshot = definitionSnapshots({
    effects: registry,
    transitions: empty,
    layerTypes: empty,
  });
  expect(snapshot[0].params).toEqual([
    { type: "gradientStops", key: "ramp", default: stops },
  ]);
});
test("agent viewer-handle shaders get a layer-coordinate mapping", () => {
  const shader = toLegacyFx(
    {
      id: "positioned",
      label: "Positioned",
      group: "Test",
      params: [
        { k: "x", label: "X", def: 0, min: 0, max: 100 },
        { k: "y", label: "Y", def: 0, min: 0, max: 100 },
      ],
      viewerHandles: [{ kind: "point", point: { x: "x", y: "y" } }],
      frag: "o=vec4(pm_layerPoint(v_px),0.,1.);",
    },
    "#version 300 es\nprecision highp float;\nin vec2 v_px;\nout vec4 o;",
  ).frag;
  expect(shader).toContain("uniform mat3 u_layerUV;");
  expect(shader).toContain("vec2 pm_layerPoint(vec2 p)");
  expect(shader).toContain("q.xy/q.z*u_layerRes");
});
import { extensionLayerFragment } from "../src/frozen/host/extension-layers";
test("agent fragment layers can declare stop collections and get the sampler", () => {
  const definition = {
    id: "agent-layer",
    label: "Agent Layer",
    version: 1,
    params: [{ k: "stops", label: "Stops", type: "gradientStops", def: stops }],
    renderer: {
      kind: "fragment",
      fragment: "void main(){fragColor=pm_stops(uv.x);} ",
    },
  };
  const shader = extensionLayerFragment(
    definition,
    "#version 300 es\nprecision highp float;\nin vec2 v_uv;\n#define uv v_uv\nout vec4 fragColor;",
  );
  expect(shader).toContain("vec4 pm_stops(float t)");
  expect(shader).toContain("uniform vec4 u_stopsC15;");
});
test("unrelated registries can retain object-shaped params", () => {
  expect(() =>
    validateViewerHandles({ id: "command", params: { label: "value" } }),
  ).not.toThrow();
});

import { frozen } from "../scripts/freeze-host";
test("native shader snapshots regenerate with the viewer additions intact", async () => {
  for (const name of ["glsl", "extension-layers"])
    expect(await frozen(name)).toBe(
      await Bun.file(
        new URL(`../src/frozen/host/${name}.js`, import.meta.url),
      ).text(),
    );
});
test("generated viewer coordinates contain real GLSL line breaks", () => {
  const def = {
    id: "mapped",
    label: "Mapped",
    group: "Test",
    params: [],
    viewerHandles: [{ kind: "point", point: { x: "x", y: "y" } }],
    frag: "o=vec4(pm_layerPoint(v_px),0.,1.);",
  };
  const source = toLegacyFx(
    def,
    "#version 300 es\nprecision highp float;\nin vec2 v_px;\nout vec4 o;",
  ).frag;
  expect(source).toContain("\nuniform mat3 u_layerUV;");
  expect(source).not.toContain(String.raw`\n`);
});
