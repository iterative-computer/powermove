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

/** The content fields an edit command writes (`c.` prefixes dropped). */
export function commandFields(command: any): string[] {
  return command.type === 'set_content' ? Object.keys(command.patch || {})
    : ['set_property', 'replace_keyframes', 'set_expression'].includes(command.type)
      ? [String(command.path || command.channel || '').replace(/^c\./, '')] : [];
}

/** pm-edit's kernels (engine-doc/edits.ts) for a layer of the open composition, or null. */
export function anchorKernel(PM: any, layer: any): { edits: any; index: number } | null {
  const edits = PM.EngineDoc?.edits;
  const index = edits ? edits.index(layer) : -1;
  return edits && index >= 0 ? { edits, index } : null;
}

export function captureFontAnchor(PM: any, layer: any, command: any) {
  if (layer?.type !== 'text' || !PM.raster) return;
  let fields = commandFields(command);
  // Changed alignment moves the origin. Repeating a static alignment must
  // retain typography anchoring, including patches that also change size.
  if (fields.includes('align')) {
    const alignment = layer.d.align;
    const current = typeof alignment === 'string' ? alignment
      : alignment && !alignment.kf?.length && !alignment.expr ? alignment.v : undefined;
    const next = command.type === 'set_content' ? command.patch.align : command.value;
    const unchanged = ['set_property', 'set_content'].includes(command.type)
      && command.mode !== 'keyframe'
      && typeof next === 'string' && current === next;
    if (!unchanged) { delete layer.d.fontAnchorBounds; return; }
    // pm-edit's kernel drops the bounds for any `align` field; an unchanged
    // alignment is not an alignment change.
    fields = fields.filter(key => key !== 'align');
  }
  if (layer.d.fontAnchorBounds) return;
  if (!fields.some(key => ['font', 'weight', 'size', 'tracking', 'leading', 'italic', 'boxWidth', 'boxHeight'].includes(key) || key.startsWith('fontAxis.'))) return;
  const time = command.time ?? PM.time;
  const legacy = () => {
    const measuredLayer = layer.d.animators?.length ? { ...layer, d: { ...layer.d, animators: [] } } : layer;
    const bounds = PM.raster(measuredLayer, 1, time)?.selection;
    if (bounds) layer.d.fontAnchorBounds = { x0: bounds.x0, y0: bounds.y0, x1: bounds.x1, y1: bounds.y1 };
  };
  const kernel = anchorKernel(PM, layer);
  if (kernel) kernel.edits.edit('captureFontAnchor', { layer: kernel.index, fields, t: time }, legacy, () => undefined);
  else legacy();
}

/** The raster selection of a text layer with recorded font bounds (the font
 *  anchor applied), before an anchor edit; null when nothing is recorded. */
export function fontAnchorReference(PM: any, layer: any, time: number): any {
  const legacy = () => {
    const raster = PM.raster?.(layer, 1, time);
    return raster?.fontOffset && raster.selection ? raster.selection : null;
  };
  const edits = PM.EngineDoc?.edits;
  return edits ? edits.anchorReference(layer, 'font', time, legacy) : legacy();
}

/**
 * After an anchor edit (Pan Behind): rescale the recorded bounds (`kind`)
 * around the new anchor so the artwork stays where `bounds` had it.
 */
export function rescaleAnchorBounds(PM: any, layer: any, kind: 'font' | 'size', time: number, bounds: any): void {
  const member = kind === 'font' ? 'fontAnchorBounds' : 'sizeAnchorBounds';
  const legacy = () => {
    const ref = layer.d[member];
    for (const axis of ['x', 'y'] as const) {
      const start = `${axis}0` as const, end = `${axis}1` as const;
      const span = bounds[end] - bounds[start], referenceSpan = ref[end] - ref[start];
      if (span <= 0) continue;
      const anchor = PM.ev(layer, 'anchor.' + axis, time);
      ref[start] = anchor - (anchor - bounds[start]) / span * referenceSpan;
      ref[end] = ref[start] + referenceSpan;
    }
  };
  const kernel = anchorKernel(PM, layer);
  if (!kernel) { legacy(); return; }
  kernel.edits.edit('anchorMoved', { layer: kernel.index, kind, t: time, bounds: { x0: bounds.x0, y0: bounds.y0, x1: bounds.x1, y1: bounds.y1 } }, legacy, () => undefined);
}
