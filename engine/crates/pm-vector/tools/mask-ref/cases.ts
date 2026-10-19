/**
 * Coverage canvases of today's bezier path mask route (`applyPathMasks` in
 * legacy/gl/compositor.ts; docs/engine/composite.md "Path masks"), for
 * pm-render's port. Deterministic: the same list every run.
 *
 * A case is the coverage canvas's size, the layer's scaled world matrix,
 * and its enabled masks with evaluated values: what `applyPathMasks`
 * receives once the compositor has evaluated the layer.
 */
export type Vertex = { x: number; y: number; inX: number; inY: number; outX: number; outY: number };
export type MaskPath = {
  x: number; y: number; rotation: number; scaleX: number; scaleY: number;
  vertices: Vertex[];
};
export type Mask = {
  mode: 'add' | 'subtract';
  shape: 'rect' | 'ellipse';
  p: { x: number; y: number; w: number; h: number; rotation: number; feather: number };
  path?: MaskPath;
};
export type Case = { id: string; width: number; height: number; world: [number, number, number, number, number, number]; masks: Mask[] };

const v = (x: number, y: number, inX = 0, inY = 0, outX = 0, outY = 0): Vertex => ({ x, y, inX, inY, outX, outY });

/** The corpus blob (engine/corpus/src/projects/compclose.ts). */
function blob(extra: Partial<MaskPath> = {}): MaskPath {
  return {
    x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100,
    vertices: [v(-60, -20, 0, 0, 30, -40), v(30, -50, -20, 0, 40, 15), v(70, 15, 0, -30, -10, 45), v(-15, 55, 40, 0, -50, 0)],
    ...extra,
  };
}

const square = (s: number): MaskPath => ({ x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, vertices: [v(-s, -s), v(s, -s), v(s, s), v(-s, s)] });

function pathMask(path: MaskPath, feather: number, at: [number, number] = [0, 0], mode: Mask['mode'] = 'add', rotation = 0): Mask {
  return { mode, shape: 'rect', p: { x: at[0], y: at[1], w: 100, h: 100, rotation, feather }, path };
}
function param(shape: Mask['shape'], x: number, y: number, w: number, h: number, feather: number, mode: Mask['mode'] = 'add', rotation = 0): Mask {
  return { mode, shape, p: { x, y, w, h, rotation, feather } };
}

const id = (s: string) => s.replace(/[^a-z0-9.-]+/gi, '-');

export function cases(): Case[] {
  const out: Case[] = [];
  const I: Case['world'] = [1, 0, 0, 1, 0, 0];
  // Feather sweep: a blob in the middle of a corpus-sized canvas. Feather is
  // CSS blur(feather · |world x| / 2 px), so σ = feather / 2 here; it covers
  // the identity threshold, the single 2D pass (σ ≤ 2/3), the two 1D passes
  // (σ ≤ 4), and every count of downscale steps.
  for (const feather of [0, 0.04, 0.06, 0.3, 0.6, 1, 1.3, 2, 3, 4.5, 6, 7.9, 8, 8.1, 9, 12, 16, 20, 24, 31, 32, 33, 48, 64, 100, 160]) {
    out.push({ id: id(`sweep-f${feather}`), width: 480, height: 300, world: [1, 0, 0, 1, 240, 150], masks: [pathMask(blob(), feather)] });
  }
  // Odd canvas sizes and fractional placement.
  for (const feather of [0, 3, 12, 24]) {
    out.push({ id: id(`odd-f${feather}`), width: 161, height: 97, world: [1, 0, 0, 1, 80.3, 47.6], masks: [pathMask(blob({ scaleX: 70, scaleY: 70 }), feather)] });
  }
  // Masks crossing the canvas edges (the decal boundary of the blur).
  for (const feather of [2, 10, 24, 60]) {
    out.push({ id: id(`edge-f${feather}`), width: 320, height: 200, world: [1, 0, 0, 1, 20, 190], masks: [pathMask(blob(), feather)] });
    out.push({ id: id(`edge-far-f${feather}`), width: 320, height: 200, world: [1, 0, 0, 1, 318, 6], masks: [pathMask(blob({ scaleX: 150, scaleY: 150 }), feather)] });
  }
  // Rotated and scaled worlds (feather scales with |world x|).
  const rot = (deg: number, sx: number, sy: number, tx: number, ty: number): Case['world'] => {
    const r = deg * Math.PI / 180;
    return [Math.cos(r) * sx, Math.sin(r) * sx, -Math.sin(r) * sy, Math.cos(r) * sy, tx, ty];
  };
  for (const [deg, sx, sy, feather] of [[-15, 1.3, 0.9, 10], [30, 0.5, 0.5, 8], [72, 2, 2, 3], [0, 0.9, 0.9, 12]] as const) {
    out.push({ id: id(`world-r${deg}-s${sx}x${sy}-f${feather}`), width: 400, height: 240, world: rot(deg, sx, sy, 200, 120), masks: [pathMask(blob(), feather)] });
  }
  // Combinations on one layer: every enabled mask is drawn in order.
  out.push({ id: 'combo-path-subtract-ellipse', width: 480, height: 300, world: [1, 0, 0, 1, 180, 20], masks: [pathMask(blob(), 16, [75, 60]), param('ellipse', 80, 60, 40, 40, 4, 'subtract')] });
  out.push({ id: 'combo-three', width: 480, height: 300, world: [1, 0, 0, 1, 340, 20], masks: [pathMask(blob({ scaleX: 105 }), 6, [70, 60]), pathMask(square(15), 2, [70, 60], 'subtract'), param('rect', 20, 20, 30, 30, 0)] });
  out.push({ id: 'combo-three-added', width: 480, height: 300, world: [1, 0, 0, 1, 340, 20], masks: [pathMask(blob({ scaleX: 105 }), 6, [70, 60]), pathMask(square(15), 2, [70, 60], 'add'), param('rect', 20, 20, 30, 30, 0)] });
  out.push({ id: 'subtract-only', width: 480, height: 300, world: rot(-15, 1.3, 0.9, 70, 230), masks: [pathMask(blob({ scaleX: 60, scaleY: 60 }), 10, [0, 0], 'subtract')] });
  out.push({ id: 'subtract-only-sharp', width: 480, height: 300, world: [1, 0, 0, 1, 240, 150], masks: [pathMask(blob(), 0, [0, 0], 'subtract'), pathMask(square(30), 0, [100, 40], 'subtract')] });
  out.push({ id: 'many-parametric', width: 400, height: 240, world: [1, 0, 0, 1, 0, 0], masks: [
    pathMask(blob(), 4, [200, 120]),
    ...Array.from({ length: 10 }, (_, i) => param(i % 2 ? 'ellipse' : 'rect', 30 + i * 36, 120 + (i % 2 ? -40 : 40), 30, 30, i % 3 * 3, i % 4 === 3 ? 'subtract' : 'add', i * 9)),
  ] });
  out.push({ id: 'signed-rect', width: 300, height: 200, world: [1, 0, 0, 1, 150, 100], masks: [pathMask(square(5), 0, [0, 0], 'add'), param('rect', 0, 0, -120, 80, 6)] });
  out.push({ id: 'mask-rotation', width: 300, height: 200, world: [1, 0, 0, 1, 150, 100], masks: [pathMask(blob({ rotation: 25, x: 10, y: -5, scaleX: 80, scaleY: 120 }), 5, [8, 4], 'add', 40)] });
  // Large canvases (UHD export sizes, the downscale path at scale).
  out.push({ id: 'hd-f24', width: 1920, height: 1080, world: [4, 0, 0, 4, 960, 540], masks: [pathMask(blob(), 24)] });
  out.push({ id: 'hd-f6-subtract', width: 1920, height: 1080, world: [2.5, 0, 0, 2.5, 900, 500], masks: [param('rect', 0, 0, 500, 300, 0), pathMask(blob(), 6, [0, 0], 'subtract')] });
  out.push({ id: 'uhd-f40', width: 3840, height: 2160, world: [6, 0, 0, 6, 1920, 1080], masks: [pathMask(blob(), 40)] });
  // Identity world, mask placed in layer pixels (a full-frame layer).
  out.push({ id: 'identity-f12', width: 480, height: 300, world: I, masks: [pathMask(blob(), 12, [240, 150])] });
  return out;
}
