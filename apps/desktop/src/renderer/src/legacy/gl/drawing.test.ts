import { it, expect } from "vitest";
import {
  gradientParameter,
  gradientSampler,
  sampleGradient,
  linearBurnPixel,
  triangleCapPoints,
} from "./drawing";
it("maps normalized linear, radial, angular and diamond paint coordinates", () => {
  expect(gradientParameter("linear", 0.25, 0.8)).toBe(0.25);
  expect(gradientParameter("radial", 1, 0.5)).toBe(1);
  expect(gradientParameter("diamond", 0.75, 0.75)).toBe(1);
  expect(gradientParameter("angular", 0.5, 1)).toBe(0.25);
});
it("interpolates arbitrary alpha stops in premultiplied space and clamps endpoints", () => {
  const s = [
    { position: 0, color: "#FF000000" },
    { position: 0.5, color: "#00FF0080" },
    { position: 1, color: "#0000FFFF" },
  ];
  expect(sampleGradient(s, 0.25)).toEqual([0, 255, 0, 64]);
  expect(sampleGradient(s, -1)).toEqual([0, 0, 0, 0]);
  expect(sampleGradient(s, 2)).toEqual([0, 0, 255, 255]);
});

it("composites linear-burn text paints with straight alpha, including transparent pixels", () => {
  expect(linearBurnPixel([150, 100, 60, 255], [120, 80, 200, 255])).toEqual([
    15, 0, 5, 255,
  ]);
  expect(linearBurnPixel([100, 80, 60, 0], [120, 80, 200, 255])).toEqual([
    120, 80, 200, 255,
  ]);
  expect(linearBurnPixel([100, 80, 60, 0], [120, 80, 200, 0])).toEqual([
    0, 0, 0, 0,
  ]);
});

it.each([0.5, 4, 17])(
  "equilateral arrow heads have three equal sides at stroke width %s",
  (width) => {
    const points = triangleCapPoints("ARROW_EQUILATERAL", width);
    const sides = points.map((p, i) =>
      Math.hypot(
        p[0] - points[(i + 1) % 3]![0],
        p[1] - points[(i + 1) % 3]![1],
      ),
    );
    expect(sides[0]).toBeCloseTo(sides[1]!, 10);
    expect(sides[1]).toBeCloseTo(sides[2]!, 10);
    expect(points[0]).toEqual([0, 0]);
    expect(points[1]![0]).toBeLessThan(0);
  },
);

it("uses Figma's unassociated color interpolation for transparent diamond stops", () => {
  const stops = [
    { position: 0, color: "#FF000000" },
    { position: 1, color: "#00FF0080" },
  ];
  expect(gradientSampler(stops, false)(0.5)).toEqual([127.5, 127.5, 0, 64]);
  expect(gradientSampler(stops)(0.5)).toEqual([0, 255, 0, 64]);
});

it("matches the five-width equilateral head from a real Figma strokeGeometry export", () => {
  const points = triangleCapPoints("ARROW_EQUILATERAL", 8);
  expect(points[1]![0]).toBe(-40);
  expect(points[2]![0]).toBe(-40);
  expect(points[2]![1] - points[1]![1]).toBeCloseTo(46.188021535, 8);
});
