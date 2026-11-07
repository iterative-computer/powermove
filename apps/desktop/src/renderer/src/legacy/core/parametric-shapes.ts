import { isProperty } from './content-properties';
import { fontAnchorOffset } from './font-anchor';

export type ShapeHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';
type Point = { x: number; y: number };
type Matrix = readonly number[];
export type ShapeResizeContext = {
  time: number;
  /** Supply the same time and gesture-start layer snapshot throughout a drag. */
  evaluate?: (layer: any, path: string, time: number) => number;
  /** Composition transform of the layer's parent/group space, before its own transform. */
  parentMatrix?: Matrix;
};
const directions: Record<ShapeHandle, Point> = {
  nw: { x: -1, y: -1 }, n: { x: 0, y: -1 }, ne: { x: 1, y: -1 }, e: { x: 1, y: 0 },
  se: { x: 1, y: 1 }, s: { x: 0, y: 1 }, sw: { x: -1, y: 1 }, w: { x: -1, y: 0 },
};

/** Data-only adapter for the existing canvas tool's normalized 100px primitives.
 * Other command origins retain their explicitly requested transform scale. */
export function drawnShapeCommand(command: any, origin: string) {
  if (origin !== 'canvas' || command.type !== 'add_layer' || command.layerType !== 'shape'
      || command.content?.paths?.length || command.content?.w !== 100 || command.content?.h !== 100) return command;
  const p = command.properties || {}, sx = p['scale.x'] ?? 100, sy = p['scale.y'] ?? 100;
  if (!(Number.isFinite(sx) && Number.isFinite(sy) && sx > 0 && sy > 0)
      || p.rotation || p.skew || p['anchor.x'] || p['anchor.y']) return command;
  return { ...command,
    content: { ...command.content, w: sx, h: sy,
      radius: (command.content.radius || 0) * Math.min(sx, sy) / 100 },
    properties: { ...p, 'scale.x': 100, 'scale.y': 100 },
  };
}

/** Constant-size geometry calculation: no renderer calls, scene scans or keyframe copies.
 * Delta is measured from the gesture start, in composition pixels. Returns four
 * standard edit commands; their auto mode keys already animated channels at time.
 * Apply these together through Edit.apply inside one live editing transaction.
 */
export function resizeParametricShape(layer: any, handle: ShapeHandle, delta: Point,
  modifiers: { shift?: boolean; alt?: boolean } = {}, context: ShapeResizeContext = { time: 0 }) {
  if (layer?.type !== 'shape' || layer.d?.paths?.length || layer.d?.shape === 'line' || layer.threeD) return null;
  // Expressions own their output; changing the base value would not resize it.
  if ([layer.d.w, layer.d.h, layer.p['position.x'], layer.p['position.y']].some(prop => prop?.expr)) return null;
  const direction = directions[handle];
  if (!direction || ![delta.x, delta.y, context.time].every(Number.isFinite)) return null;
  const read = (path: string, fallback = 0): number => {
    const prop = path.startsWith('c.') ? layer.d[path.slice(2)] : layer.p[path];
    if (prop == null) return fallback;
    // Animated channels must use the editor's evaluator, never a guessed base value.
    if (isProperty(prop) && (prop.kf.length || prop.expr) && !context.evaluate) return NaN;
    return Number(context.evaluate ? context.evaluate(layer, path, context.time) : isProperty(prop) ? prop.v : prop);
  };
  const w = read('c.w'), h = read('c.h');
  const px = read('position.x'), py = read('position.y'), ax = read('anchor.x'), ay = read('anchor.y');
  const rotation = read('rotation') * Math.PI / 180, skew = Math.tan(read('skew') * Math.PI / 180);
  const sx = read('scale.x', 100) / 100, sy = read('scale.y', 100) / 100;
  const cos = Math.cos(rotation), sin = Math.sin(rotation);
  const a = cos * sx, b = sin * sx, c = (cos * skew - sin) * sy, d = (sin * skew + cos) * sy;
  const parent = context.parentMatrix ?? [1, 0, 0, 1, 0, 0];
  const wa = parent[0]! * a + parent[2]! * b, wb = parent[1]! * a + parent[3]! * b;
  const wc = parent[0]! * c + parent[2]! * d, wd = parent[1]! * c + parent[3]! * d;
  const det = wa * wd - wb * wc;
  if (![w, h, px, py, ax, ay, a, b, c, d, det].every(Number.isFinite)
      || w <= 0 || h <= 0 || Math.abs(det) < 1e-10) return null;
  const dx = (wd * delta.x - wc * delta.y) / det, dy = (-wb * delta.x + wa * delta.y) / det;
  const factor = modifiers.alt ? 2 : 1;
  let width = Math.max(0.01, w + direction.x * dx * factor);
  let height = Math.max(0.01, h + direction.y * dy * factor);
  if (modifiers.shift) {
    const rx = width / w, ry = height / h;
    const ratio = !direction.x ? ry : !direction.y ? rx : Math.abs(rx - 1) >= Math.abs(ry - 1) ? rx : ry;
    const safeRatio = Math.max(0.01 / w, 0.01 / h, ratio);
    width = w * safeRatio; height = h * safeRatio;
  }
  // The command engine captures sizeAnchorBounds on its first content edit.
  // Predict that capture so non-centred anchors retain the same fixed edge.
  const reference = layer.d.sizeAnchorBounds ?? { x0: -w / 2, y0: -h / 2, x1: w / 2, y1: h / 2 };
  const oldOffset = fontAnchorOffset(reference, { x0: -w / 2, y0: -h / 2, x1: w / 2, y1: h / 2 }, ax, ay);
  const nextOffset = fontAnchorOffset(reference, { x0: -width / 2, y0: -height / 2, x1: width / 2, y1: height / 2 }, ax, ay);
  const mx = (modifiers.alt ? 0 : direction.x * (width - w) / 2) + oldOffset.x - nextOffset.x;
  const my = (modifiers.alt ? 0 : direction.y * (height - h) / 2) + oldOffset.y - nextOffset.y;
  const position = { x: px + a * mx + c * my, y: py + b * mx + d * my };
  const commands = [['c.w', width], ['c.h', height], ['position.x', position.x], ['position.y', position.y]].map(([path, value]) => ({
    type: 'set_property' as const, target: layer.id, path: path as string, value: value as number, mode: 'auto' as const, time: context.time,
  }));
  return { width, height, position, commands };
}
