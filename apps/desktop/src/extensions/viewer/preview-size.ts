/* Pure preview sizing, shared by the live viewer and its Library card. Kept
   free of the viewer runtime so the card can import it without loading it. */
export function previewRenderSize(width: number, height: number, zoom: number, dpr: number, quality: number) {
  const safeWidth = Math.max(2, Number(width) || 2);
  const safeHeight = Math.max(2, Number(height) || 2);
  const displayScale = Math.min(1, Math.max(0.05, Number(zoom) || 0.05) * Math.min(2, Math.max(1, Number(dpr) || 1)));
  const scale = Math.max(0.05, Math.min(1, displayScale * Math.max(0.25, Math.min(1, Number(quality) || 1))));
  return {
    width: Math.max(2, Math.round(safeWidth * scale)),
    height: Math.max(2, Math.round(safeHeight * scale)),
    scale,
  };
}
