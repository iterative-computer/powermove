/*
 * Level 1 SVG buckets of the engine's viewer against today's viewer
 * (decision S1; docs/engine/vector.md, "Level 1 SVG images").
 *
 * Today's viewer draws a view into a canvas it sizes itself
 * (`previewRenderSize`, the explicit preview resolution, or the cropped
 * `previewRenderViewport`), and the compositor picks each SVG image's bucket
 * at that canvas's output scale (`svgRasterDimensions(…, scaledWorld(L, T,
 * W, H))`, `outputScale(W, H)`). The engine used to draw the view at zoom
 * × DPR; it now renders at today's canvas (`todaysCanvas`, sent as
 * `ViewerState.canvas`; decision V1), so every bucket is picked at today's
 * output scale (`canvasOutputScale`).
 *
 * This test computes today's buckets with today's functions over a grid of
 * zoom × DPR × layer scale (also preview quality, crop safety, and an SVG
 * inside a precomp), checks the scale the viewer sends gives them, and
 * keeps the fixture the engine's test reads
 * (engine/crates/pm-render/tests/fixtures/svg-viewer-buckets.json,
 * `svg_level1.rs`) equal to them. Regenerate it with
 * `PM_UPDATE_SVG_BUCKETS=1 bunx vitest run src/extensions/viewer/svg-buckets.test.ts`.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import { svgRasterDimensions } from '../../renderer/src/legacy/gl/compositor';
import { compositionFramePosition, previewRenderViewport, viewerCanvasSize } from './viewer';
import { canvasOutputScale, todaysCanvas as viewerCanvas } from './engine-present/view-params';

const FIXTURE = path.resolve(__dirname, '../../../../../engine/crates/pm-render/tests/fixtures/svg-viewer-buckets.json');

/** The e2e viewer (`e2e/engine-svg.spec.ts`): its stage and pan. */
const STAGE = { width: 748, height: 452 };
const PAN = { x: -140, y: 60 };
const COMP = { w: 600, h: 338 };
/** The SVG asset's natural size and the layer's box (contain). */
const SOURCE = { w: 160, h: 90 };
const BOX = { w: 320, h: 180 };
/** A precomp layer the size of the composition over a smaller one. */
const SUB = { w: 400, h: 226 };

const ZOOMS = [0.1, 0.17, 0.25, 0.33, 0.5, 0.61, 0.75, 0.9, 1, 1.14375, 1.25, 1.5, 2, 2.37, 3, 4, 6, 8];
const DPRS = [1, 2, 3];
const LAYER_SCALES: [number, number][] = [[0.35, 0.35], [1, 1], [2.6, 2.6], [1.5, 0.6]];
/** Auto (quality 1) and the explicit Half menu item. */
const QUALITIES = [{ resolution: 'auto', quality: 1 }, { resolution: '0.5', quality: 0.5 }] as const;

/** Today's `outputScale(W, H)` (legacy/gl/compositor.ts), its own code:
 *  the crop's scale when the cropped preview viewport is active, the
 *  canvas over the composition otherwise. */
function compositorOutputScale(W: number, H: number, viewport: { width: number; height: number } | null, comp: { w: number; h: number }): [number, number] {
  if (viewport) return [W / Math.max(1e-6, viewport.width), H / Math.max(1e-6, viewport.height)];
  return [W / Math.max(1, Number(comp?.w) || W), H / Math.max(1, Number(comp?.h) || H)];
}

/** Today's viewer canvas for a view (`V.layout`, through its own
 *  `viewerCanvasSize`): the crop when the project is crop safe and zoomed
 *  in, else the explicit resolution or `previewRenderSize`. */
function todaysCanvas(zoom: number, devicePixelRatio: number, resolution: string, quality: number, cropSafe: boolean) {
  const dpr = Math.min(devicePixelRatio || 1, 2);
  const position = compositionFramePosition(STAGE, { width: COMP.w, height: COMP.h }, zoom, PAN);
  const viewport = cropSafe
    ? previewRenderViewport(COMP.w, COMP.h, zoom, STAGE.width, STAGE.height, position.x, position.y, dpr, quality, 128, null)
    : null;
  const size = viewerCanvasSize(COMP.w, COMP.h, zoom, dpr, quality, resolution, viewport);
  return { width: size.width, height: size.height, viewport };
}

/** The output scale of the canvas the viewer sends for a view. */
function sent(canvas: { width: number; height: number }, viewport: { x: number; y: number; width: number; height: number } | null): [number, number] {
  return canvasOutputScale(viewerCanvas(COMP.w, COMP.h, canvas, viewport, { x: 0, y: 0 }, 1)!);
}

type Row = {
  name: string;
  zoom: number; dpr: number; resolution: string; cropSafe: boolean; nested: boolean;
  layerScale: [number, number];
  /** Today's canvas and whether it was cropped. */
  canvas: [number, number]; cropped: boolean;
  /** The output scale of the canvas the viewer sends (`todaysCanvas`). */
  outputScale: [number, number];
  /** Today's bucket. */
  today: [number, number];
};

function rows(): Row[] {
  const out: Row[] = [];
  for (const zoom of ZOOMS) for (const dpr of DPRS) for (const { resolution, quality } of QUALITIES) {
    for (const cropSafe of [true, false]) for (const layerScale of LAYER_SCALES) {
      const canvas = todaysCanvas(zoom, dpr, resolution, quality, cropSafe);
      const [sx, sy] = compositorOutputScale(canvas.width, canvas.height, canvas.viewport, COMP);
      const outputScale = sent(canvas, canvas.viewport);
      const bucket = svgRasterDimensions(SOURCE.w, SOURCE.h, BOX.w, BOX.h, [layerScale[0] * sx, 0, 0, layerScale[1] * sy, 0, 0]);
      out.push({
        name: `zoom ${zoom} dpr ${dpr} ${resolution} ${cropSafe ? 'crop-safe' : 'crop-unsafe'} layer ${layerScale.join('x')}`,
        zoom, dpr, resolution, cropSafe, nested: false, layerScale,
        canvas: [canvas.width, canvas.height], cropped: !!canvas.viewport,
        outputScale, today: [bucket.width, bucket.height]
      });
    }
    // The SVG inside a precomp (a precomp layer is never crop safe): the
    // child renders into `round(w × sx)` × `round(h × sy)` and its layers
    // take that target over the child's size (`renderProject(sub, …,
    // targetW, targetH)`, `outputScale` inside it).
    if (resolution !== 'auto') continue;
    const layerScale: [number, number] = [1.7, 1.7];
    const canvas = todaysCanvas(zoom, dpr, resolution, quality, false);
    const [sx, sy] = compositorOutputScale(canvas.width, canvas.height, null, COMP);
    const targetW = Math.max(2, Math.round(COMP.w * sx)), targetH = Math.max(2, Math.round(COMP.h * sy));
    const [ix, iy] = compositorOutputScale(targetW, targetH, null, SUB);
    const bucket = svgRasterDimensions(SOURCE.w, SOURCE.h, BOX.w, BOX.h, [layerScale[0] * ix, 0, 0, layerScale[1] * iy, 0, 0]);
    out.push({
      name: `zoom ${zoom} dpr ${dpr} nested layer ${layerScale.join('x')}`,
      zoom, dpr, resolution, cropSafe: false, nested: true, layerScale,
      canvas: [canvas.width, canvas.height], cropped: false,
      outputScale: sent(canvas, null), today: [bucket.width, bucket.height]
    });
  }
  return out;
}

describe('level 1 SVG buckets in the engine viewer (today’s rule)', () => {
  const grid = rows();

  it('today’s viewer caps the SVG scale at one pixel per composition pixel below fit', () => {
    // The perf3 review's case: 61% at DPR 2 (1.22 device pixels per
    // composition pixel) draws today at scale 1, not 1.22.
    const canvas = todaysCanvas(0.61, 2, 'auto', 1, true);
    expect(canvas.viewport).toBeNull();
    expect(sent(canvas, null)).toEqual([1, 1]);
    // Cropped at 237%: the crop's own rounding, not zoom × DPR.
    const cropped = todaysCanvas(2.37, 2, 'auto', 1, true);
    expect(cropped.viewport).not.toBeNull();
    const [sx] = sent(cropped, cropped.viewport);
    expect(sx).toBeCloseTo(4.74, 2);
  });

  it('the canvas the viewer sends has today’s compositor output scale', () => {
    expect(grid.length).toBeGreaterThan(800);
    for (const row of grid.filter((r) => !r.nested)) {
      const canvas = todaysCanvas(row.zoom, row.dpr, row.resolution, row.resolution === 'auto' ? 1 : 0.5, row.cropSafe);
      expect(row.outputScale, row.name).toEqual(compositorOutputScale(canvas.width, canvas.height, canvas.viewport, COMP));
    }
  });

  it('no scale for a canvas it cannot describe', () => {
    expect(viewerCanvas(600, 338, { width: Number.NaN, height: 2 }, null, { x: 0, y: 0 }, 1)).toBeNull();
    expect(viewerCanvas(0, 0, { width: 300, height: 169 }, null, { x: 0, y: 0 }, 1)).toBeNull();
  });

  it('the engine’s fixture is today’s buckets', () => {
    // One row per line; the name is the test's message only.
    const head = JSON.stringify({
      comment: 'Generated by apps/desktop/src/extensions/viewer/svg-buckets.test.ts from today’s viewer and compositor functions; read by svg_level1.rs.',
      comp: COMP, sub: SUB, source: SOURCE, box: BOX
    });
    const text = `${head.slice(0, -1)},"rows":[\n${grid.map(({ name: _name, ...row }) => JSON.stringify(row)).join(',\n')}\n]}\n`;
    if (process.env.PM_UPDATE_SVG_BUCKETS === '1') writeFileSync(FIXTURE, text);
    expect(readFileSync(FIXTURE, 'utf8')).toBe(text);
  });
});
