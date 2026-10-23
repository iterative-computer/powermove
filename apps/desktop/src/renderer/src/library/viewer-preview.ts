const finitePositive = (value: number): boolean => Number.isFinite(value) && value > 0;

/**
 * Where today's composition canvas sits on a stage-level engine
 * presentation: the viewer tags its presentation canvas with the box and
 * backing size of today's canvas for the frame on screen
 * (`data-todays-canvas="x y width height backingWidth backingHeight"`, stage
 * CSS px; extensions/viewer/viewer.ts). Null for a presentation without it
 * (a canvas bound inside a viewer's own stage-inner already has today's box).
 */
function compositionBox(original: HTMLCanvasElement) {
  const fields = original.dataset['todaysCanvas']?.split(' ').map(Number);
  if (!fields || fields.length !== 6) return null;
  const [left, top, width, height, renderWidth, renderHeight] = fields as [number, number, number, number, number, number];
  if (![width, height, renderWidth, renderHeight].every(finitePositive) || !Number.isFinite(left) || !Number.isFinite(top)) return null;
  const box = original.getBoundingClientRect();
  if (!finitePositive(box.width) || !finitePositive(box.height)) return null;
  return {
    left, top, width, height,
    renderWidth: Math.round(renderWidth),
    renderHeight: Math.round(renderHeight),
    // Presentation backing px per CSS px.
    scaleX: original.width / box.width,
    scaleY: original.height / box.height,
  };
}

/** Copy the pixels already on screen into a Library clone. Native presentation
 * has a percentage-sized CSS box and must
 * retain the source box in CSS pixels: the card has different dimensions,
 * while its stage-inner and overlay still use the live stage coordinates.
 *
 * A stage-level presentation covers the whole stage, with the composition
 * drawn where today's WebGL canvas sits. Scaled into the card, the
 * composition's edges would then be filtered inside the bitmap, where today's
 * card shows the edge of its own canvas element. When the viewer tagged where
 * today's canvas sits, the copy is that composition canvas, at today's box
 * and backing size, so the card's edges are today's.
 * The caller invokes any panel-specific library.render after this copy. */
export function copyClonedCanvasBitmaps(source: Element, target: Element): void {
  const from = source.querySelectorAll<HTMLCanvasElement>('canvas');
  target.querySelectorAll<HTMLCanvasElement>('canvas').forEach((canvas, index) => {
    const original = from[index];
    if (!original?.width || !original.height) return;
    // Keep legacy and custom canvases on their original Library copy path.
    // The native stage surface alone needs its source-stage coordinate box.
    if (original.id === 'engine-present') {
      const frame = compositionBox(original);
      if (frame) {
        Object.assign(canvas.style, { inset: 'auto', left: `${frame.left}px`, top: `${frame.top}px`, width: `${frame.width}px`, height: `${frame.height}px` });
        canvas.width = frame.renderWidth;
        canvas.height = frame.renderHeight;
        try {
          canvas.getContext('2d')?.drawImage(original,
            frame.left * frame.scaleX, frame.top * frame.scaleY, frame.width * frame.scaleX, frame.height * frame.scaleY,
            0, 0, frame.renderWidth, frame.renderHeight);
        } catch { /* tainted or GL-only canvas */ }
        return;
      }
      const style = getComputedStyle(original);
      const width = parseFloat(style.width), height = parseFloat(style.height);
      if (width > 0 && Number.isFinite(width)) canvas.style.width = `${width}px`;
      if (height > 0 && Number.isFinite(height)) canvas.style.height = `${height}px`;
    }
    canvas.width = original.width;
    canvas.height = original.height;
    try { canvas.getContext('2d')?.drawImage(original, 0, 0); } catch { /* tainted or GL-only canvas */ }
  });
}

/** Panel-authored Library cards retain control after their live pixels copy. */
export function renderLibraryCallback(
  library: { render?: (context: { source: HTMLElement; clone: HTMLElement; width: number; height: number }) => void } | null | undefined,
  context: { source: HTMLElement; clone: HTMLElement; width: number; height: number },
): boolean {
  if (typeof library?.render !== 'function') return false;
  library.render(context);
  return true;
}
