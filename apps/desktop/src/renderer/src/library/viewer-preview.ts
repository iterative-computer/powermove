/** Copy the pixels already on screen into a Library clone. Native presentation
 * has a percentage-sized CSS box and must
 * retain the source box in CSS pixels: the card has different dimensions,
 * while its stage-inner and overlay still use the live stage coordinates.
 * The caller invokes any panel-specific library.render after this copy. */
export function copyClonedCanvasBitmaps(source: Element, target: Element): void {
  const from = source.querySelectorAll<HTMLCanvasElement>('canvas');
  target.querySelectorAll<HTMLCanvasElement>('canvas').forEach((canvas, index) => {
    const original = from[index];
    if (!original?.width || !original.height) return;
    // Keep legacy and custom canvases on their original Library copy path.
    // The native stage surface alone needs its source-stage coordinate box.
    if (original.id === 'engine-present') {
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
