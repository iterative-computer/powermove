import { it, expect } from "vitest";
import { makePM } from "../../__tests__/make-pm";
import { sanitizeProject } from "../../../core/validate/project";
import { convertFigma } from "../../../../../shared/figma/convert";
import {
  encodeProjectContainer,
  decodeProjectContainer,
} from "../../../../../shared/project-container";
import { allFeatures } from "../../../../../shared/figma/__fixtures__/all-features";
import { materializeFigma, applyFigmaPlan } from "./convert";
function editor() {
  const P = makePM(
    "core/easing",
    "core/model",
    "core/selection",
    "core/anim",
    "core/history",
    "core/editing",
  );
  P.proj = P.mkProject({ w: 930, h: 680, dur: 8 });
  P.mkEffect = (type: string, version: number) => ({
    id: P.uid("fx"),
    type,
    version,
    on: true,
    p: {},
  });
  return P;
}
it("round-trips gradients, rich text, real-image transforms, stroke styles and nested mattes in a pmv container", () => {
  const P = editor(),
    d = allFeatures();
  d.nodes = d.nodes.filter(
    (n) => !["Glass effect", "Texture effect"].includes(n.name),
  );
  const plan = materializeFigma(P, convertFigma(d, P.proj), 0);
  applyFigmaPlan(P, plan);
  const gradient = P.proj.layers.find((l: any) => l.name === "LINEAR gradient");
  expect(
    P.Edit.apply({
      type: "set_property",
      target: gradient.id,
      path: "c.fillGradient.stops.1.position",
      value: 0.6,
      time: 1,
      preserveHandEdits: false,
    }).ok,
  ).toBe(true);
  P.setKeyOn(P.findProp(gradient, "c.fillGradient.stops.1.position"), 2, 0.8);
  const arc = P.proj.layers.find((l: any) => l.name === "Arc donut");
  expect(
    P.Edit.apply({
      type: "set_property",
      target: arc.id,
      path: "c.arc.inner",
      value: 0.25,
      time: 1,
    }).ok,
  ).toBe(true);
  const saved = JSON.parse(P.serialize()),
    document = decodeProjectContainer(
      encodeProjectContainer(saved, []),
    ).document,
    restored: any = sanitizeProject(document.proj);
  expect(
    restored.layers.find((l: any) => l.name === "LINEAR gradient")!.d
      .fillGradient,
  ).toEqual(gradient.d.fillGradient);
  expect(
    restored.layers.find((l: any) => l.name === "Styled text")!.d.runs,
  ).toEqual(P.proj.layers.find((l: any) => l.name === "Styled text").d.runs);
  expect(
    restored.layers.find((l: any) => l.name === "Image TILE fill-0")!.d
      .imageMapping,
  ).toEqual({
    mode: "tile",
    transform: [1, 0, 0, 1, 0.1, 0],
    scale: 0.5,
    rotation: 0,
  });
  expect(
    restored.layers.find((l: any) => l.name === "Inner shadow")!.fx,
  ).toEqual(P.proj.layers.find((l: any) => l.name === "Inner shadow").fx);
  expect(
    restored.layers.find((l: any) => l.name === "Arrow caps")!.d.strokeStyle,
  ).toEqual(
    P.proj.layers.find((l: any) => l.name === "Arrow caps").d.strokeStyle,
  );
  expect(restored.layers.map((l: any) => l.matteSources)).toEqual(
    P.proj.layers.map((l: any) => l.matteSources),
  );
});
it("re-sync preserves nested drawing animation and hand-added effects", () => {
  const P = editor(),
    d = allFeatures();
  d.nodes = [d.nodes[0]!];
  const apply = (update = false) =>
    applyFigmaPlan(P, materializeFigma(P, convertFigma(d, P.proj), 0), update);
  apply();
  const l = P.proj.layers[0];
  P.Edit.apply({
    type: "set_property",
    target: l.id,
    path: "c.fillGradient.stops.1.position",
    value: 0.6,
    time: 0,
  });
  P.setKeyOn(P.findProp(l, "c.fillGradient.stops.1.position"), 1, 0.8);
  const keys = JSON.stringify(l.d.fillGradient.stops[1].position);
  d.nodes[0]!.fills[0]!.gradientStops![1]!.position = 0.2;
  d.nodes[0]!.fills[0]!.gradientStops![0]!.color = { r: 0, g: 0, b: 1, a: 0.3 };
  apply(true);
  expect(JSON.stringify(l.d.fillGradient.stops[1].position)).toBe(keys);
  expect(l.d.fillGradient.stops[0].color).toBe("#0000FF4D");
});
it("opens older solid shape, point text, media and blend defaults unchanged", () => {
  const P = editor();
  for (const type of ["shape", "text", "image"])
    P.proj.layers.push(P.mkLayer(type));
  const saved = JSON.parse(P.serialize()).proj;
  const restored: any = sanitizeProject(saved);
  expect(restored.layers.map((l: any) => l.type)).toEqual([
    "shape",
    "text",
    "image",
  ]);
  expect(restored.layers[0]!.d.fillGradient).toBeUndefined();
  expect(restored.layers[1]!.d.richText).toBeUndefined();
  expect(restored.layers[2]!.d.imageMapping).toBeUndefined();
});

it("evaluates nested gradient colors with alpha instead of holding them as strings", () => {
  const P = editor(),
    d = allFeatures();
  d.nodes = [d.nodes[0]!];
  applyFigmaPlan(P, materializeFigma(P, convertFigma(d, P.proj), 0));
  const l = P.proj.layers[0];
  expect(
    P.Edit.apply({
      type: "set_property",
      target: l.id,
      path: "c.fillGradient.stops.0.color",
      value: "#FF000000",
      time: 0,
    }).ok,
  ).toBe(true);
  const prop = P.findProp(l, "c.fillGradient.stops.0.color");
  P.setKeyOn(prop, 0, "#FF000000");
  P.setKeyOn(prop, 1, "#0000FFFF");
  expect(P.evP(l, prop, 0.5, "c.fillGradient.stops.0.color")).toBe("#80008080");
});

it("keeps missing fonts editable and returns a host warning while recognizing bundled fonts", () => {
  const P = editor(),
    d = allFeatures();
  d.nodes = [structuredClone(d.nodes.find((n) => n.name === "Styled text")!)];
  P.Fonts = { families: ["Arial"], bundled: ["Bundled Face"] };
  d.nodes[0]!.text!.runs[0]!.font = "Unavailable Face";
  d.nodes[0]!.text!.runs[1]!.font = "Bundled Face";
  const p = materializeFigma(P, convertFigma(d, P.proj), 0);
  expect(p.layers[0].type).toBe("text");
  expect(p.layers[0].d.runs[0].font).toBe("Unavailable Face");
  expect(p.warnings?.map((w) => w.reason)).toEqual([
    "Missing font: Unavailable Face",
  ]);
  applyFigmaPlan(P, p);
  expect(
    P.Edit.apply({
      type: "set_property",
      target: p.layers[0].id,
      path: "c.runs.0.start",
      value: 2,
    }).ok,
  ).toBe(false);
});

it("removes obsolete imported paint data during re-sync without removing authored animators", () => {
  const P = editor(),
    d = allFeatures();
  d.nodes = [d.nodes[0]!];
  applyFigmaPlan(P, materializeFigma(P, convertFigma(d, P.proj), 0));
  const l = P.proj.layers[0];
  l.d.animators = [{ id: "authored", mode: "wave", p: {} }];
  expect(l.d.fillGradient).toBeDefined();
  expect(l.figmaSource.importedContentKeys).toContain("fillGradient");
  d.nodes[0]!.fills = [{ type: "SOLID", color: { r: 0, g: 0, b: 1 } }];
  applyFigmaPlan(P, materializeFigma(P, convertFigma(d, P.proj), 0), true);
  expect(l.d.fillGradient).toBeUndefined();
  expect(l.d.color).toBe("#0000FFFF");
  expect(l.d.animators[0].id).toBe("authored");
  const saved: any = sanitizeProject(JSON.parse(P.serialize()).proj);
  expect(saved.layers[0].figmaSource.importedContentKeys).toEqual(
    l.figmaSource.importedContentKeys,
  );
});
