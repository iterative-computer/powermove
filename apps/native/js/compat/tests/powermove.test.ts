import { expect, test } from "bun:test";
import { CompatRealm, powermove } from "../src";
import { readFontAxes } from "../src/frozen/typography/font-catalog";
import { FakeHost } from "./fake-host";
test("vector factories, evaluation and content helpers retain property references", () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "paths",
      generation: 1,
      host,
      initial: host.publication(),
    });
  const PM = realm.PM;
  PM.evP = (_layer: any, prop: any) => prop.v;
  const path = powermove.makeVectorPath(PM),
    vertex = powermove.makeVertex(PM, 3, 4);
  path.vertices.push(vertex);
  const layer = { type: "shape", d: { paths: [path], color: PM.P("#ff0000") } };
  const props = powermove.structuredProperties(layer);
  expect(
    props.find((p) => p.key === `g.${path.id}.v.${vertex.id}.x`)!.prop,
  ).toBe(vertex.p.x);
  expect(powermove.pathValues(PM, layer, path, 0).vertices[0]).toMatchObject({
    x: 3,
    y: 4,
  });
  expect(powermove.resolveContent(PM, layer, 0).color).toBe("#ff0000");
  expect(layer.d.color).toBeInstanceOf(Object);
  expect(powermove.canAnimateContent(layer, "color")).toBe(true);
});
test("time-remap helper uses structural lease, live services and host touch reentry", () => {
  const host = new FakeHost({
    id: "remap",
    name: "remap",
    w: 1920,
    h: 1080,
    fps: 30,
    layers: [
      { id: "video", type: "video", from: 0, dur: 2, d: { trim: 0, speed: 1 } },
    ],
    comps: {},
    revision: 0,
    edits: [],
  });
  const realm = new CompatRealm({
      id: "remap",
      generation: 1,
      host,
      initial: host.publication(),
    }),
    PM = realm.PM;
  PM.time = 0.5;
  PM.invalidate = () => {};
  PM.setKeyOn = (prop: any, t: number, v: any) => {
    const key = { t, v, i: PM.uid("k") };
    prop.kf.push(key);
    return key;
  };
  let refreshed = 0,
    revealed = 0;
  realm.services.register("timeline", { reveal: () => revealed++ });
  realm.services.register("inspector", { refresh: () => refreshed++ });
  powermove.enableTimeRemap(PM, PM.L("video"), "freeze");
  expect(PM.L("video").d.sourceTime.v).toBe(0.5);
  expect(PM.L("video").d.sourceTime.kf[0].hold).toBe(true);
  expect(refreshed).toBe(1);
  expect(revealed).toBe(1);
  expect(PM.hist.canUndo()).toBe(true);
});
test("matte chains reject cycles; text unit/stagger helpers preserve graphemes", () => {
  const a = { id: "a", type: "shape", matteSource: "b" },
    b = { id: "b", type: "shape", matteSource: "a" };
  expect(powermove.validMatteSource([a, b], a, "b")).toBe(false);
  expect(powermove.validMatteSource([a, b], a, null)).toBe(true);
  expect(powermove.countTextUnits([{ text: "e\u0301 👋🏽" }], "characters")).toBe(
    2,
  );
  expect(powermove.countTextUnits([{ text: "one two" }], "words")).toBe(2);
  expect(
    powermove.staggerLength({ duration: 0.5, stagger: 0.1 }, 3),
  ).toBeCloseTo(0.7);
});
test("font axis parser validates bounded font records without DOM", () => {
  const bytes = new Uint8Array(68),
    view = new DataView(bytes.buffer);
  view.setUint32(0, 0x10000);
  view.setUint16(4, 1);
  bytes.set(
    [..."fvar"].map((c) => c.charCodeAt(0)),
    12,
  );
  view.setUint32(20, 32);
  view.setUint32(24, 36);
  view.setUint16(32, 1);
  view.setUint16(36, 16);
  view.setUint16(40, 1);
  view.setUint16(42, 20);
  bytes.set(
    [..."wght"].map((c) => c.charCodeAt(0)),
    48,
  );
  view.setInt32(52, 100 * 65536);
  view.setInt32(56, 400 * 65536);
  view.setInt32(60, 900 * 65536);
  expect(readFontAxes(bytes.buffer)).toEqual([
    { tag: "wght", label: "Weight", min: 100, max: 900, default: 400 },
  ]);
  expect(readFontAxes(bytes.buffer.slice(0, 20))).toEqual([]);
});
