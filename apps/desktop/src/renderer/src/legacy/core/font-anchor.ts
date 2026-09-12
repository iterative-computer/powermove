/** Keep the anchor at the same relative point as typography changes its bounds. */
export function fontAnchorOffset(reference: any, bounds: any, ax: number, ay: number) {
  if (!reference || !bounds) return { x: 0, y: 0 };
  const offset = (start: number, end: number, nextStart: number, nextEnd: number, anchor: number) => {
    if (![start, end, nextStart, nextEnd, anchor].every(Number.isFinite) || end <= start) return 0;
    return anchor - (nextStart + (anchor - start) / (end - start) * (nextEnd - nextStart));
  };
  return { x: offset(reference.x0, reference.x1, bounds.x0, bounds.x1, ax),
    y: offset(reference.y0, reference.y1, bounds.y0, bounds.y1, ay) };
}

export function captureFontAnchor(PM: any, layer: any, command: any) {
  if (layer?.type !== 'text' || layer.d.fontAnchorBounds || !PM.raster) return;
  const fields = command.type === 'set_content' ? Object.keys(command.patch || {})
    : ['set_property', 'replace_keyframes', 'set_expression'].includes(command.type)
      ? [String(command.path || command.channel || '').replace(/^c\./, '')] : [];
  if (!fields.some(key => ['font', 'weight', 'size', 'tracking', 'leading', 'align', 'italic', 'boxWidth', 'boxHeight'].includes(key) || key.startsWith('fontAxis.'))) return;
  const measuredLayer = layer.d.animators?.length ? { ...layer, d: { ...layer.d, animators: [] } } : layer;
  const bounds = PM.raster(measuredLayer, 1, command.time ?? PM.time)?.selection;
  if (bounds) layer.d.fontAnchorBounds = { x0: bounds.x0, y0: bounds.y0, x1: bounds.x1, y1: bounds.y1 };
}
