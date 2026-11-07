export type VisualBounds = { x0: number; y0: number; x1: number; y1: number };

/** Transform source bounds into composition space; includes rotation, skew and reflection. */
export function transformedBounds(bounds: VisualBounds, matrix: readonly number[]): VisualBounds {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const x of [bounds.x0, bounds.x1]) for (const y of [bounds.y0, bounds.y1]) {
    const px = matrix[0]! * x + matrix[2]! * y + matrix[4]!;
    const py = matrix[1]! * x + matrix[3]! * y + matrix[5]!;
    x0 = Math.min(x0, px); y0 = Math.min(y0, py);
    x1 = Math.max(x1, px); y1 = Math.max(y1, py);
  }
  return { x0, y0, x1, y1 };
}

/** Outward rounding retains every edge pixel. Non-rendering rig layers do not enlarge the crop. */
export function selectionBounds(PM: any, layers: any[], time: number) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  PM.beginEval?.(time);
  for (const layer of layers) {
    if (layer.type === 'group' || PM.TYPE_META[layer.type]?.visual === false) continue;
    // A smaller viewport changes 3D camera projection. Do not silently alter a 3D scene.
    const seen = new Set();
    for (let item = layer; item && !seen.has(item); item = PM.L?.(item.parent)) {
      seen.add(item);
      if (item.threeD || (PM.groupAncestors?.(item) || []).some((group: any) => group.threeD)
          || /^powermove\.3d\./.test(item.d?.definition || '')) {
        throw new Error('Size to selection currently supports 2D layers.');
      }
    }
    const source = PM.GL?.bounds?.(layer, time);
    if (!source) throw new Error('Could not measure the selected layer.');
    const bounds = transformedBounds(source, PM.worldMatrix(layer, time));
    if (!Object.values(bounds).every(Number.isFinite)) throw new Error('The selection has invalid bounds.');
    x0 = Math.min(x0, bounds.x0); y0 = Math.min(y0, bounds.y0);
    x1 = Math.max(x1, bounds.x1); y1 = Math.max(y1, bounds.y1);
  }
  if (!Number.isFinite(x0)) throw new Error('Select a visual layer to size the pre-comp.');
  x0 = Math.floor(x0); y0 = Math.floor(y0); x1 = Math.ceil(x1); y1 = Math.ceil(y1);
  return { x0, y0, x1, y1, w: Math.max(2, x1 - x0), h: Math.max(2, y1 - y0) };
}
