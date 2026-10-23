/**
 * WebGL clears a canvas's drawing buffer after compositing unless it was
 * created with preserveDrawingBuffer. Keep a separate bitmap at the viewport
 * overlay's draw boundary so a Library clone can copy exactly what was drawn.
 * The source canvas remains the live, unmodified WebGL presentation surface.
 */
const retained = new WeakMap<HTMLCanvasElement, HTMLCanvasElement>();

/** The returned canvas is owned by the overlay; callers must copy it now. */
export function getRetainedViewportOverlayBitmap(source: HTMLCanvasElement): HTMLCanvasElement | null {
  const bitmap = retained.get(source);
  return bitmap && bitmap.width === source.width && bitmap.height === source.height ? bitmap : null;
}

/** Call synchronously after the last WebGL draw, before browser compositing. */
export function retainViewportOverlayBitmap(source: HTMLCanvasElement): void {
  if (!source.width || !source.height) {
    retained.delete(source);
    return;
  }
  const bitmap = retained.get(source) ?? source.ownerDocument.createElement('canvas');
  if (bitmap.width !== source.width) bitmap.width = source.width;
  if (bitmap.height !== source.height) bitmap.height = source.height;
  const context = bitmap.getContext('2d');
  if (!context) {
    retained.delete(source);
    return;
  }
  try {
    // A new transparent frame must erase the previous frame's opaque pixels.
    context.clearRect(0, 0, bitmap.width, bitmap.height);
    context.drawImage(source, 0, 0);
    retained.set(source, bitmap);
  } catch {
    // Never return a partial or old frame after a context/readback failure.
    retained.delete(source);
  }
}

export function releaseViewportOverlayBitmap(source: HTMLCanvasElement): void {
  retained.delete(source);
}
