import { it, expect } from "vitest";
import { allFeatures } from "../../../../../shared/figma/__fixtures__/all-features";
import {
  convertFigma,
  unsupportedReasons,
} from "../../../../../shared/figma/convert";
import { clipboardDoc } from "./adapter";
import { figmaNode } from "./__fixtures__/clipboard";
const fixture = allFeatures();
it("extracts actual image bytes by kiwi dataBlob, not the selection preview", () => {
  const png = Uint8Array.from(Buffer.from(fixture.images.checker!, "base64"));
  const n = figmaNode(1, "ELLIPSE", {
    fillPaints: [
      {
        type: "IMAGE",
        image: { hash: Uint8Array.from([1, 2, 3]), dataBlob: 1 },
        imageScaleMode: "STRETCH",
        transform: { m00: 1, m01: 0, m02: 0.1, m10: 0, m11: 1, m12: 0 },
      },
    ],
  });
  const d = clipboardDoc({ nodes: [n], blobs: [Uint8Array.from([99]), png] });
  expect(d.images["010203"]).toBe(fixture.images.checker);
  expect(d.nodes[0]!.fills[0]).toMatchObject({
    imageHash: "010203",
    scaleMode: "CROP",
    imageTransform: [1, 0, 0, 1, 0.1, 0],
  });
  expect(convertFigma(d, { w: 100, h: 100 }).issues).toEqual([]);
});
it("resolves clipboard styled text overrides by stable style ID", () => {
  const n = figmaNode(1, "TEXT", {
    textData: {
      characters: "Ab",
      characterStyleIDs: [0, 9],
      styleOverrideTable: [
        {
          styleID: 9,
          fontSize: 32,
          fontName: { family: "Arial", style: "Bold" },
          letterSpacing: { units: "PIXELS", value: 2 },
          textDecoration: "UNDERLINE",
        } as any,
      ],
    },
  });
  const d = clipboardDoc({ nodes: [n], blobs: [] });
  expect(d.nodes[0]!.text!.runs[1]).toMatchObject({
    start: 1,
    end: 2,
    size: 32,
    tracking: 2,
    decoration: "UNDERLINE",
  });
  expect(unsupportedReasons(d.nodes[0]!)).toEqual([]);
});

it("uses clipboard computed stroke geometry when scalar width is absent", () => {
  const bytes = new Uint8Array(28),
    v = new DataView(bytes.buffer);
  let i = 0;
  for (const [op, x, y] of [
    [1, 0, 0],
    [2, 100, 0],
    [2, 0, 60],
  ]) {
    bytes[i++] = op!;
    v.setFloat32(i, x!, true);
    i += 4;
    v.setFloat32(i, y!, true);
    i += 4;
  }
  bytes[i] = 0;
  const n = figmaNode(1, "VECTOR", {
    fillPaints: [],
    strokePaints: [{ type: "SOLID", color: { r: 0, g: 0, b: 1, a: 1 } }],
    strokeGeometry: [{ commandsBlob: 0, windingRule: "NONZERO" }],
  });
  const d = clipboardDoc({ nodes: [n], blobs: [bytes] });
  expect(d.nodes[0]!.strokePaths).toHaveLength(1);
  expect(convertFigma(d, { w: 100, h: 100 }).issues).toEqual([]);
});
it("keeps all subpaths of one kiwi winding region together so holes survive", () => {
  const bytes: number[] = [];
  const point = (op: number, x: number, y: number) => {
    bytes.push(op);
    const b = new Uint8Array(8),
      v = new DataView(b.buffer);
    v.setFloat32(0, x, true);
    v.setFloat32(4, y, true);
    bytes.push(...b);
  };
  for (const [x, y, w, h] of [
    [0, 0, 90, 60],
    [20, 15, 50, 30],
  ]) {
    point(1, x!, y!);
    point(2, x! + w!, y!);
    point(2, x! + w!, y! + h!);
    point(2, x!, y! + h!);
    bytes.push(0);
  }
  const raw = figmaNode(1, "VECTOR", {
    fillGeometry: [{ commandsBlob: 0, windingRule: "ODD" }],
  });
  const d = clipboardDoc({ nodes: [raw], blobs: [Uint8Array.from(bytes)] });
  expect(d.nodes[0]!.paths).toHaveLength(1);
  expect(d.nodes[0]!.paths![0]!.data.match(/M /g)).toHaveLength(2);
  expect(d.nodes[0]!.paths![0]!.windingRule).toBe("EVENODD");
  const paths = convertFigma(d, { w: 100, h: 100 }).commands[0]!.content.paths;
  expect(paths.map((p: any) => p.fillGroup)).toEqual(["0", "0"]);
});
