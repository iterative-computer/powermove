import { expect, it } from "vitest";
import { FIGMA_DRAWING_EFFECTS } from "./figma-drawing";

it("uses the clamped finite background kernel for new imports and preserves revision 1", () => {
  const blur = FIGMA_DRAWING_EFFECTS.find(
    (effect) => effect.id === "figmaBackdropBlur",
  )!;
  expect(blur.version).toBe(2);
  expect(blur.frag).toContain(
    "clamp(uv+axis*d/u_layerRes,.5/u_layerRes,1.-.5/u_layerRes)",
  );
  expect(blur.legacy?.[0]?.version).toBe(1);
  expect(blur.legacy?.[0]?.frag).toContain("abs(d)>sigma*3.");
  expect(blur.legacy?.[0]?.frag).not.toContain("clamp(uv+axis*d/u_layerRes");
  const layerBlur = FIGMA_DRAWING_EFFECTS.find(
    (effect) => effect.id === "figmaLayerBlur",
  )!;
  expect(layerBlur.version).toBe(1);
  expect(layerBlur.frag).not.toContain("clamp(uv+axis*d/u_layerRes");
});
