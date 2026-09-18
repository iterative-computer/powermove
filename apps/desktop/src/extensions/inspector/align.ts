/* Alignment math for the inspector strip. World-space boxes move so one of
   their edges or centers meets the target frame; the caller converts each
   delta into the layer's parent space and writes position channels. */

export type AlignMode = 'left' | 'hcenter' | 'right' | 'top' | 'vcenter' | 'bottom';

export interface Box { x0: number; y0: number; x1: number; y1: number }

export const ALIGN_LABELS: Record<AlignMode, string> = {
  left: 'Align left', hcenter: 'Align horizontal centers', right: 'Align right',
  top: 'Align top', vcenter: 'Align vertical centers', bottom: 'Align bottom',
};

export function unionBox(boxes: Box[]): Box | null {
  if (!boxes.length) return null;
  return {
    x0: Math.min(...boxes.map(b => b.x0)), y0: Math.min(...boxes.map(b => b.y0)),
    x1: Math.max(...boxes.map(b => b.x1)), y1: Math.max(...boxes.map(b => b.y1)),
  };
}

/** World-space translation that aligns `box` to `frame` for `mode`. */
export function alignDelta(box: Box, frame: Box, mode: AlignMode): { dx: number; dy: number } {
  switch (mode) {
    case 'left': return { dx: frame.x0 - box.x0, dy: 0 };
    case 'right': return { dx: frame.x1 - box.x1, dy: 0 };
    case 'hcenter': return { dx: (frame.x0 + frame.x1) / 2 - (box.x0 + box.x1) / 2, dy: 0 };
    case 'top': return { dx: 0, dy: frame.y0 - box.y0 };
    case 'bottom': return { dx: 0, dy: frame.y1 - box.y1 };
    case 'vcenter': return { dx: 0, dy: (frame.y0 + frame.y1) / 2 - (box.y0 + box.y1) / 2 };
  }
}

/** A world delta expressed in a parent's local axes (translation ignored). */
export function deltaInParent(parent: readonly number[] | null | undefined, dx: number, dy: number): { dx: number; dy: number } {
  if (!parent || parent.length < 4) return { dx, dy };
  const [a, b, c, d] = parent as [number, number, number, number];
  const det = a * d - b * c;
  if (Math.abs(det) < 1e-9) return { dx, dy };
  return { dx: (d * dx - c * dy) / det, dy: (a * dy - b * dx) / det };
}

/** One layer aligns to the composition; several align to their common bounds (Figma). */
export function alignFrame(boxes: Box[], composition: { w: number; h: number }): Box | null {
  if (boxes.length > 1) return unionBox(boxes);
  return { x0: 0, y0: 0, x1: composition.w, y1: composition.h };
}
