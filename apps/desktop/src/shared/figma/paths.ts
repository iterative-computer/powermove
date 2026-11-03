export type Point = { x: number; y: number };
export type Vertex = Point & { inX: number; inY: number; outX: number; outY: number };
const COMMAND = /^[a-z]$/i;
function vertex(x: number, y: number): Vertex {
  return { x, y, inX: 0, inY: 0, outX: 0, outY: 0 };
}

function addLine(vertices: Vertex[], x: number, y: number) {
  if (!vertices.length) vertices.push(vertex(x, y));
  else vertices.push(vertex(x, y));
}

function addCubic(vertices: Vertex[], c1: Point, c2: Point, end: Point) {
  const start = vertices.at(-1);
  if (!start) {
    vertices.push(vertex(end.x, end.y));
    return;
  }
  start.outX = c1.x - start.x;
  start.outY = c1.y - start.y;
  const next = vertex(end.x, end.y);
  next.inX = c2.x - end.x;
  next.inY = c2.y - end.y;
  vertices.push(next);
}

function arcCubics(
  start: Point,
  rxInput: number,
  ryInput: number,
  rotation: number,
  large: boolean,
  sweep: boolean,
  end: Point,
) {
  let rx = Math.abs(rxInput),
    ry = Math.abs(ryInput);
  if (!rx || !ry || (start.x === end.x && start.y === end.y)) return [];
  const phi = (rotation * Math.PI) / 180,
    cos = Math.cos(phi),
    sin = Math.sin(phi);
  const dx = (start.x - end.x) / 2,
    dy = (start.y - end.y) / 2;
  const x1 = cos * dx + sin * dy,
    y1 = -sin * dx + cos * dy;
  const radii = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
  if (radii > 1) {
    const scale = Math.sqrt(radii);
    rx *= scale;
    ry *= scale;
  }
  const numerator = Math.max(0, rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1);
  const denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
  const factor = (large === sweep ? -1 : 1) * Math.sqrt(denominator ? numerator / denominator : 0);
  const cx1 = (factor * rx * y1) / ry,
    cy1 = (factor * -ry * x1) / rx;
  const cx = cos * cx1 - sin * cy1 + (start.x + end.x) / 2;
  const cy = sin * cx1 + cos * cy1 + (start.y + end.y) / 2;
  const angle = (ux: number, uy: number, vx: number, vy: number) => Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
  let theta = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry);
  let delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
  if (!sweep && delta > 0) delta -= Math.PI * 2;
  if (sweep && delta < 0) delta += Math.PI * 2;
  const count = Math.max(1, Math.ceil(Math.abs(delta) / (Math.PI / 2)));
  const step = delta / count;
  const map = (x: number, y: number): Point => ({
    x: cx + cos * rx * x - sin * ry * y,
    y: cy + sin * rx * x + cos * ry * y,
  });
  const result: Array<{ c1: Point; c2: Point; end: Point }> = [];
  for (let i = 0; i < count; i++) {
    const a = theta + i * step,
      b = a + step,
      k = (4 / 3) * Math.tan((b - a) / 4);
    result.push({
      c1: map(Math.cos(a) - k * Math.sin(a), Math.sin(a) + k * Math.cos(a)),
      c2: map(Math.cos(b) + k * Math.sin(b), Math.sin(b) - k * Math.cos(b)),
      end: map(Math.cos(b), Math.sin(b)),
    });
  }
  return result;
}

export function parseSvgPathData(data: string): Array<{ vertices: Vertex[]; closed: boolean }> {
  const tokens = data.match(/[a-z]|[-+]?(?:\d*\.\d+|\d+\.?)(?:e[-+]?\d+)?/gi) || [];
  const result: Array<{ vertices: Vertex[]; closed: boolean }> = [];
  let i = 0,
    command = '',
    x = 0,
    y = 0,
    startX = 0,
    startY = 0;
  let vertices: Vertex[] = [],
    previousCubic: Point | null = null,
    previousQuadratic: Point | null = null;
  const number = () => {
    const token = tokens[i++];
    if (token == null || COMMAND.test(token)) throw new Error('Invalid SVG path data');
    return Number(token);
  };
  const hasNumbers = () => i < tokens.length && !COMMAND.test(tokens[i]!);
  const finish = (closed = false) => {
    if (closed && vertices.length > 1) {
      const first = vertices[0]!,
        last = vertices.at(-1)!;
      if (Math.abs(first.x - last.x) < 1e-8 && Math.abs(first.y - last.y) < 1e-8) {
        first.inX = last.inX;
        first.inY = last.inY;
        vertices.pop();
      }
    }
    if (vertices.length) result.push({ vertices, closed });
    vertices = [];
  };
  while (i < tokens.length) {
    if (COMMAND.test(tokens[i]!)) command = tokens[i++]!;
    if (!command) throw new Error('SVG path must begin with a move command');
    const relative = command === command.toLowerCase(),
      kind = command.toUpperCase();
    const point = (): Point => {
      const px = number(),
        py = number();
      return { x: relative ? x + px : px, y: relative ? y + py : py };
    };
    if (kind === 'Z') {
      finish(true);
      x = startX;
      y = startY;
      previousCubic = previousQuadratic = null;
      command = '';
      continue;
    }
    if (kind === 'M') {
      const p = point();
      if (vertices.length) finish(false);
      vertices.push(vertex(p.x, p.y));
      x = startX = p.x;
      y = startY = p.y;
      previousCubic = previousQuadratic = null;
      command = relative ? 'l' : 'L';
      continue;
    }
    if (!vertices.length) throw new Error('SVG path command before move');
    if (!hasNumbers()) throw new Error(`SVG ${kind} command is missing coordinates`);
    switch (kind) {
      case 'L': {
        const p = point();
        addLine(vertices, p.x, p.y);
        x = p.x;
        y = p.y;
        break;
      }
      case 'H': {
        x = relative ? x + number() : number();
        addLine(vertices, x, y);
        break;
      }
      case 'V': {
        y = relative ? y + number() : number();
        addLine(vertices, x, y);
        break;
      }
      case 'C': {
        const c1 = point(),
          c2 = point(),
          p = point();
        addCubic(vertices, c1, c2, p);
        x = p.x;
        y = p.y;
        previousCubic = c2;
        previousQuadratic = null;
        break;
      }
      case 'S': {
        const c1 = previousCubic ? { x: x * 2 - previousCubic.x, y: y * 2 - previousCubic.y } : { x, y };
        const c2 = point(),
          p = point();
        addCubic(vertices, c1, c2, p);
        x = p.x;
        y = p.y;
        previousCubic = c2;
        previousQuadratic = null;
        break;
      }
      case 'Q': {
        const q = point(),
          p = point();
        addCubic(
          vertices,
          { x: x + (2 / 3) * (q.x - x), y: y + (2 / 3) * (q.y - y) },
          { x: p.x + (2 / 3) * (q.x - p.x), y: p.y + (2 / 3) * (q.y - p.y) },
          p,
        );
        x = p.x;
        y = p.y;
        previousQuadratic = q;
        previousCubic = null;
        break;
      }
      case 'T': {
        const q: Point = previousQuadratic
          ? { x: x * 2 - previousQuadratic.x, y: y * 2 - previousQuadratic.y }
          : { x, y };
        const p = point();
        addCubic(
          vertices,
          { x: x + (2 / 3) * (q.x - x), y: y + (2 / 3) * (q.y - y) },
          { x: p.x + (2 / 3) * (q.x - p.x), y: p.y + (2 / 3) * (q.y - p.y) },
          p,
        );
        x = p.x;
        y = p.y;
        previousQuadratic = q;
        previousCubic = null;
        break;
      }
      case 'A': {
        const rx = number(),
          ry = number(),
          rotation = number(),
          large = !!number(),
          sweep = !!number(),
          p = point();
        const curves = arcCubics({ x, y }, rx, ry, rotation, large, sweep, p);
        if (!curves.length) addLine(vertices, p.x, p.y);
        else curves.forEach((curve) => addCubic(vertices, curve.c1, curve.c2, curve.end));
        x = p.x;
        y = p.y;
        previousCubic = curves.at(-1)?.c2 || null;
        previousQuadratic = null;
        break;
      }
      default:
        throw new Error(`Unsupported SVG path command: ${kind}`);
    }
    if (!['C', 'S'].includes(kind)) previousCubic = null;
    if (!['Q', 'T'].includes(kind)) previousQuadratic = null;
  }
  finish(false);
  return result;
}
