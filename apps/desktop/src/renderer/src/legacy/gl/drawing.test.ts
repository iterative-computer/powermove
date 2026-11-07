import { it, expect } from "vitest";
import { gradientParameter, sampleGradient, linearBurnPixel } from "./drawing";
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
