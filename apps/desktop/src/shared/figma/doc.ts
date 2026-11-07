import { z } from 'zod';

/** FigmaDoc v1. Pixels, radians only for arcs; matrix [a,b,c,d,tx,ty].
 * Nodes are bottom-to-top, transforms are absolute in the source file.
 * Paint transforms map normalized node coordinates into normalized paint space.
 * IDs remain stable across sends. Synthetic children use deterministic suffixes.
 * Raster PNGs include their absolute axis-aligned render bounds (effects included).
 * No URLs, executable code, host objects or font bytes cross this boundary.
 */
const n = z.number().finite().min(-1e7).max(1e7);
const unit = z.number().min(0).max(1);
const size = z.number().min(0).max(1e6);
export const matrixSchema = z.tuple([n, n, n, n, n, n]);
export type Matrix = z.infer<typeof matrixSchema>;
const colorSchema = z.object({ r: unit, g: unit, b: unit, a: unit.optional() }).strict();
const bytes = z
  .string()
  .max(24 * 1024 * 1024)
  .regex(/^[A-Za-z0-9+/]*={0,2}$/);
const paintSchema = z
  .object({
    type: z.string().max(80),
    visible: z.boolean().optional(),
    opacity: unit.optional(),
    color: colorSchema.optional(),
    blendMode: z.string().max(80).optional(),
    gradientTransform: matrixSchema.optional(),
    gradientStops: z
      .array(z.object({ position: unit, color: colorSchema }).strict())
      .max(5000)
      .optional(),
    imageHash: z.string().max(200).optional(),
    scaleMode: z.string().max(80).optional(),
    imageTransform: matrixSchema.optional(),
    scalingFactor: size.optional(),
    rotation: n.optional(),
  })
  .strict();
const effectSchema = z
  .object({
    type: z.string().max(80),
    visible: z.boolean().optional(),
    radius: size.optional(),
    spread: n.optional(),
    offset: z.object({ x: n, y: n }).strict().optional(),
    color: colorSchema.optional(),
    blendMode: z.string().max(80).optional(),
    blurType: z.string().max(40).optional(),
    startRadius: size.optional(),
    startOffset: z.object({ x: n, y: n }).strict().optional(),
    endOffset: z.object({ x: n, y: n }).strict().optional(),
    noiseType: z.string().max(40).optional(),
    noiseSize: size.optional(),
    density: unit.optional(),
    opacity: unit.optional(),
    secondaryColor: colorSchema.optional(),
    clipToShape: z.boolean().optional(),
    lightIntensity: unit.optional(),
    lightAngle: n.optional(),
    refraction: unit.optional(),
    depth: size.optional(),
    dispersion: unit.optional(),
  })
  .strict();
const runSchema = z
  .object({
    start: z.number().int().min(0),
    end: z.number().int().min(0),
    font: z.string().max(300),
    style: z.string().max(100),
    size: z.number().min(1).max(16384),
    weight: z.number().min(1).max(1000).optional(),
    fills: z.array(paintSchema).max(32),
    decoration: z.string().max(80).optional(),
    textCase: z.string().max(80).optional(),
    tracking: n.optional(),
    leading: size.optional(),
    paragraphSpacing: size.optional(),
    paragraphIndent: n.optional(),
    listSpacing: size.optional(),
    indentation: size.optional(),
    listType: z.string().max(40).optional(),
  })
  .strict();
const rectSchema = z.object({ x: n, y: n, w: size, h: size }).strict();
const baseNodeSchema = z.object({
  id: z.string().min(1).max(300),
  fileKey: z.string().max(300),
  type: z.string().max(80),
  name: z.string().max(1000),
  absoluteTransform: matrixSchema,
  w: size,
  h: size,
  visible: z.boolean().optional(),
  locked: z.boolean().optional(),
  opacity: unit.optional(),
  blendMode: z.string().max(80).optional(),
  fills: z.array(paintSchema).max(32).default([]),
  strokes: z.array(paintSchema).max(32).default([]),
  effects: z.array(effectSchema).max(32).default([]),
  strokeWeight: size.optional(),
  strokeAlign: z.enum(['CENTER', 'INSIDE', 'OUTSIDE']).optional(),
  dashes: z.array(size).max(100).optional(),
  cap: z.string().max(80).optional(),
  join: z.string().max(80).optional(),
  startCap: z.string().max(80).optional(),
  endCap: z.string().max(80).optional(),
  miterLimit: size.optional(),
  strokeWeights: z.tuple([size, size, size, size]).optional(),
  pointCount: z.number().int().min(3).max(1000).optional(),
  booleanOperation: z.string().max(40).optional(),
  radii: z.tuple([size, size, size, size]).optional(),
  smoothing: unit.optional(),
  clipsContent: z.boolean().optional(),
  isMask: z.boolean().optional(),
  maskType: z.string().max(80).optional(),
  paths: z
    .array(
      z
        .object({
          data: z.string().max(2e6),
          windingRule: z.string().max(80),
          startCap: z.string().max(80).optional(),
          endCap: z.string().max(80).optional(),
        })
        .strict(),
    )
    .max(1000)
    .optional(),
  strokePaths: z
    .array(
      z
        .object({
          data: z.string().max(2e6),
          windingRule: z.string().max(80),
          startCap: z.string().max(80).optional(),
          endCap: z.string().max(80).optional(),
        })
        .strict(),
    )
    .max(1000)
    .optional(),
  text: z
    .object({
      characters: z.string().max(1e6),
      runs: z.array(runSchema).max(5000),
      align: z.string().max(80).optional(),
      autoResize: z.string().max(80).optional(),
      leading: size.optional(),
      tracking: n.optional(),
      baseline: n.optional(),
      advanced: z.boolean().optional(),
      verticalAlign: z.string().max(40).optional(),
      paragraphSpacing: size.optional(),
      paragraphIndent: n.optional(),
      truncation: z.string().max(40).optional(),
      maxLines: z.number().int().min(0).max(10000).optional(),
    })
    .strict()
    .optional(),
  arc: z.object({ start: n, end: n, inner: unit }).strict().optional(),
  unsupported: z.array(z.string().max(200)).max(100).optional(),
  raster: z.object({ png: bytes, bounds: rectSchema }).strict().optional(),
});
export type FigmaNode = z.infer<typeof baseNodeSchema> & { children: FigmaNode[] };
export const nodeSchema: z.ZodType<FigmaNode> = baseNodeSchema.extend({
  children: z.lazy(() => z.array(nodeSchema).max(5000)),
});
export const docSchema = z
  .object({
    format: z.literal('powermove-figma'),
    version: z.literal(1),
    source: z.enum(['plugin', 'clipboard']),
    nodes: z.array(nodeSchema).min(1).max(5000),
    images: z.record(z.string().max(200), bytes).default({}),
  })
  .strict();
export type FigmaDoc = z.infer<typeof docSchema>;
export type Paint = z.infer<typeof paintSchema>;
export type Rect = z.infer<typeof rectSchema>;
export const FIGMA_CLIPBOARD_PREFIX = 'POWERMOVE_FIGMA_V1\n';
export const MAX_PAYLOAD = 32 * 1024 * 1024;
/** Bound depth and count BEFORE recursive schema parsing, also reject prototype keys. */
export function validateFigmaDoc(value: unknown): FigmaDoc {
  let count = 0;
  const queue: Array<[unknown, number]> = [[value, 0]];
  while (queue.length) {
    const [v, depth] = queue.pop()!;
    if (++count > 300000 || depth > 110) throw new Error('Figma document is too complex');
    if (v && typeof v === 'object')
      for (const [k, item] of Object.entries(v)) {
        if (['__proto__', 'constructor', 'prototype'].includes(k)) throw new Error('Invalid Figma document');
        queue.push([item, depth + 1]);
      }
  }
  const doc = docSchema.parse(value),
    ids = new Set<string>();
  let nodes = 0;
  const visit = (node: FigmaNode) => {
    if (++nodes > 5000 || ids.has(node.id)) throw new Error('Duplicate or too many Figma layers');
    ids.add(node.id);
    node.children.forEach(visit);
  };
  doc.nodes.forEach(visit);
  return doc;
}
export const mul = (a: Matrix, b: Matrix): Matrix => [
  a[0] * b[0] + a[2] * b[1],
  a[1] * b[0] + a[3] * b[1],
  a[0] * b[2] + a[2] * b[3],
  a[1] * b[2] + a[3] * b[3],
  a[0] * b[4] + a[2] * b[5] + a[4],
  a[1] * b[4] + a[3] * b[5] + a[5],
];
export function inverse(m: Matrix): Matrix {
  const d = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(d) < 1e-10) throw new Error('Singular Figma transform');
  return [
    m[3] / d,
    -m[1] / d,
    -m[2] / d,
    m[0] / d,
    (m[2] * m[5] - m[3] * m[4]) / d,
    (m[1] * m[4] - m[0] * m[5]) / d,
  ];
}
export function bounds(node: FigmaNode): Rect {
  const m = node.absoluteTransform;
  const p = [
    [0, 0],
    [node.w, 0],
    [0, node.h],
    [node.w, node.h],
  ].map(([x, y]) => [m[0] * x! + m[2] * y! + m[4], m[1] * x! + m[3] * y! + m[5]]);
  const x = Math.min(...p.map((v) => v[0]!)),
    y = Math.min(...p.map((v) => v[1]!));
  return { x, y, w: Math.max(...p.map((v) => v[0]!)) - x, h: Math.max(...p.map((v) => v[1]!)) - y };
}
