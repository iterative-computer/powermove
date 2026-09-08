import { makeVectorPath, makeVertex, type VectorPath } from './vector-paths';

type Point = { x: number; y: number };
type Vertex = Point & { inX: number; inY: number; outX: number; outY: number };
type Matrix = [number, number, number, number, number, number];

export type ImportedSvgPath = {
  name: string;
  vertices: Vertex[];
  closed: boolean;
  fill: string;
  fillEnabled: boolean;
  fillOpacity: number;
  stroke: string;
  strokeWidth: number;
  strokeOpacity: number;
};

export type ImportedSvg = {
  width: number;
  height: number;
  paths: ImportedSvgPath[];
  warnings: string[];
};

const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];
const COMMAND = /^[a-z]$/i;
const NUMBER = /[-+]?(?:\d*\.\d+|\d+\.?)(?:e[-+]?\d+)?/ig;

function multiply(a: Matrix, b: Matrix): Matrix {
  return [
    a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5],
  ];
}

function apply(matrix: Matrix, point: Point): Point {
  return {
    x: matrix[0] * point.x + matrix[2] * point.y + matrix[4],
    y: matrix[1] * point.x + matrix[3] * point.y + matrix[5],
  };
}

function transformList(value: string | null): Matrix {
  let result: Matrix = IDENTITY;
  const pattern = /([a-z]+)\s*\(([^)]*)\)/ig;
  for (const match of value?.matchAll(pattern) || []) {
    const values = (match[2]!.match(NUMBER) || []).map(Number);
    let next: Matrix = IDENTITY;
    switch (match[1]!.toLowerCase()) {
      case 'matrix': if (values.length >= 6) next = values.slice(0, 6) as Matrix; break;
      case 'translate': next = [1, 0, 0, 1, values[0] || 0, values[1] || 0]; break;
      case 'scale': next = [values[0] ?? 1, 0, 0, values[1] ?? values[0] ?? 1, 0, 0]; break;
      case 'rotate': {
        const radians = (values[0] || 0) * Math.PI / 180;
        const rotation: Matrix = [Math.cos(radians), Math.sin(radians), -Math.sin(radians), Math.cos(radians), 0, 0];
        if (values.length >= 3) {
          const [cx, cy] = values.slice(1);
          next = multiply(multiply([1, 0, 0, 1, cx!, cy!], rotation), [1, 0, 0, 1, -cx!, -cy!]);
        } else next = rotation;
        break;
      }
      case 'skewx': next = [1, 0, Math.tan((values[0] || 0) * Math.PI / 180), 1, 0, 0]; break;
      case 'skewy': next = [1, Math.tan((values[0] || 0) * Math.PI / 180), 0, 1, 0, 0]; break;
    }
    result = multiply(result, next);
  }
  return result;
}

function vertex(x: number, y: number): Vertex {
  return { x, y, inX: 0, inY: 0, outX: 0, outY: 0 };
}

function addLine(vertices: Vertex[], x: number, y: number) {
  if (!vertices.length) vertices.push(vertex(x, y));
  else vertices.push(vertex(x, y));
}

function addCubic(vertices: Vertex[], c1: Point, c2: Point, end: Point) {
  const start = vertices.at(-1);
  if (!start) { vertices.push(vertex(end.x, end.y)); return; }
  start.outX = c1.x - start.x;
  start.outY = c1.y - start.y;
  const next = vertex(end.x, end.y);
  next.inX = c2.x - end.x;
  next.inY = c2.y - end.y;
  vertices.push(next);
}

function arcCubics(start: Point, rxInput: number, ryInput: number, rotation: number, large: boolean, sweep: boolean, end: Point) {
  let rx = Math.abs(rxInput), ry = Math.abs(ryInput);
  if (!rx || !ry || (start.x === end.x && start.y === end.y)) return [];
  const phi = rotation * Math.PI / 180, cos = Math.cos(phi), sin = Math.sin(phi);
  const dx = (start.x - end.x) / 2, dy = (start.y - end.y) / 2;
  const x1 = cos * dx + sin * dy, y1 = -sin * dx + cos * dy;
  const radii = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
  if (radii > 1) { const scale = Math.sqrt(radii); rx *= scale; ry *= scale; }
  const numerator = Math.max(0, rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1);
  const denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
  const factor = (large === sweep ? -1 : 1) * Math.sqrt(denominator ? numerator / denominator : 0);
  const cx1 = factor * rx * y1 / ry, cy1 = factor * -ry * x1 / rx;
  const cx = cos * cx1 - sin * cy1 + (start.x + end.x) / 2;
  const cy = sin * cx1 + cos * cy1 + (start.y + end.y) / 2;
  const angle = (ux: number, uy: number, vx: number, vy: number) => Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
  let theta = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry);
  let delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
  if (!sweep && delta > 0) delta -= Math.PI * 2;
  if (sweep && delta < 0) delta += Math.PI * 2;
  const count = Math.max(1, Math.ceil(Math.abs(delta) / (Math.PI / 2)));
  const step = delta / count;
  const map = (x: number, y: number): Point => ({ x: cx + cos * rx * x - sin * ry * y, y: cy + sin * rx * x + cos * ry * y });
  const result: Array<{ c1: Point; c2: Point; end: Point }> = [];
  for (let i = 0; i < count; i++) {
    const a = theta + i * step, b = a + step, k = 4 / 3 * Math.tan((b - a) / 4);
    result.push({
      c1: map(Math.cos(a) - k * Math.sin(a), Math.sin(a) + k * Math.cos(a)),
      c2: map(Math.cos(b) + k * Math.sin(b), Math.sin(b) - k * Math.cos(b)),
      end: map(Math.cos(b), Math.sin(b)),
    });
  }
  return result;
}

export function parseSvgPathData(data: string): Array<{ vertices: Vertex[]; closed: boolean }> {
  const tokens = data.match(/[a-z]|[-+]?(?:\d*\.\d+|\d+\.?)(?:e[-+]?\d+)?/ig) || [];
  const result: Array<{ vertices: Vertex[]; closed: boolean }> = [];
  let i = 0, command = '', x = 0, y = 0, startX = 0, startY = 0;
  let vertices: Vertex[] = [], previousCubic: Point | null = null, previousQuadratic: Point | null = null;
  const number = () => { const token = tokens[i++]; if (token == null || COMMAND.test(token)) throw new Error('Invalid SVG path data'); return Number(token); };
  const hasNumbers = () => i < tokens.length && !COMMAND.test(tokens[i]!);
  const finish = (closed = false) => {
    if (closed && vertices.length > 1) {
      const first = vertices[0]!, last = vertices.at(-1)!;
      if (Math.abs(first.x - last.x) < 1e-8 && Math.abs(first.y - last.y) < 1e-8) {
        first.inX = last.inX; first.inY = last.inY; vertices.pop();
      }
    }
    if (vertices.length) result.push({ vertices, closed });
    vertices = [];
  };
  while (i < tokens.length) {
    if (COMMAND.test(tokens[i]!)) command = tokens[i++]!;
    if (!command) throw new Error('SVG path must begin with a move command');
    const relative = command === command.toLowerCase(), kind = command.toUpperCase();
    const point = (): Point => { const px = number(), py = number(); return { x: relative ? x + px : px, y: relative ? y + py : py }; };
    if (kind === 'Z') {
      finish(true); x = startX; y = startY; previousCubic = previousQuadratic = null; command = '';
      continue;
    }
    if (kind === 'M') {
      const p = point();
      if (vertices.length) finish(false);
      vertices.push(vertex(p.x, p.y)); x = startX = p.x; y = startY = p.y;
      previousCubic = previousQuadratic = null;
      command = relative ? 'l' : 'L';
      continue;
    }
    if (!vertices.length) throw new Error('SVG path command before move');
    if (!hasNumbers()) throw new Error(`SVG ${kind} command is missing coordinates`);
    switch (kind) {
      case 'L': { const p = point(); addLine(vertices, p.x, p.y); x = p.x; y = p.y; break; }
      case 'H': { x = relative ? x + number() : number(); addLine(vertices, x, y); break; }
      case 'V': { y = relative ? y + number() : number(); addLine(vertices, x, y); break; }
      case 'C': {
        const c1 = point(), c2 = point(), p = point(); addCubic(vertices, c1, c2, p);
        x = p.x; y = p.y; previousCubic = c2; previousQuadratic = null; break;
      }
      case 'S': {
        const c1 = previousCubic ? { x: x * 2 - previousCubic.x, y: y * 2 - previousCubic.y } : { x, y };
        const c2 = point(), p = point(); addCubic(vertices, c1, c2, p);
        x = p.x; y = p.y; previousCubic = c2; previousQuadratic = null; break;
      }
      case 'Q': {
        const q = point(), p = point();
        addCubic(vertices, { x: x + 2 / 3 * (q.x - x), y: y + 2 / 3 * (q.y - y) }, { x: p.x + 2 / 3 * (q.x - p.x), y: p.y + 2 / 3 * (q.y - p.y) }, p);
        x = p.x; y = p.y; previousQuadratic = q; previousCubic = null; break;
      }
      case 'T': {
        const q: Point = previousQuadratic ? { x: x * 2 - previousQuadratic.x, y: y * 2 - previousQuadratic.y } : { x, y };
        const p = point();
        addCubic(vertices, { x: x + 2 / 3 * (q.x - x), y: y + 2 / 3 * (q.y - y) }, { x: p.x + 2 / 3 * (q.x - p.x), y: p.y + 2 / 3 * (q.y - p.y) }, p);
        x = p.x; y = p.y; previousQuadratic = q; previousCubic = null; break;
      }
      case 'A': {
        const rx = number(), ry = number(), rotation = number(), large = !!number(), sweep = !!number(), p = point();
        const curves = arcCubics({ x, y }, rx, ry, rotation, large, sweep, p);
        if (!curves.length) addLine(vertices, p.x, p.y); else curves.forEach(curve => addCubic(vertices, curve.c1, curve.c2, curve.end));
        x = p.x; y = p.y; previousCubic = curves.at(-1)?.c2 || null; previousQuadratic = null; break;
      }
      default: throw new Error(`Unsupported SVG path command: ${kind}`);
    }
    if (!['C', 'S'].includes(kind)) previousCubic = null;
    if (!['Q', 'T'].includes(kind)) previousQuadratic = null;
  }
  finish(false);
  return result;
}

function styleMap(element: Element) {
  const result: Record<string, string> = {};
  for (const pair of (element.getAttribute('style') || '').split(';')) {
    const colon = pair.indexOf(':');
    if (colon > 0) result[pair.slice(0, colon).trim().toLowerCase()] = pair.slice(colon + 1).trim();
  }
  return result;
}

function normalizedColor(input: string | null, fallback: string) {
  const value = String(input || fallback).trim().toLowerCase();
  const named: Record<string, string> = { black: '#000000', white: '#FFFFFF', red: '#FF0000', green: '#008000', blue: '#0000FF', transparent: '#000000' };
  if (named[value]) return named[value];
  if (/^#[0-9a-f]{3}$/i.test(value)) return ('#' + [...value.slice(1)].map(c => c + c).join('')).toUpperCase();
  if (/^#[0-9a-f]{6}$/i.test(value)) return value.toUpperCase();
  if (/^#[0-9a-f]{8}$/i.test(value)) return value.slice(0, 7).toUpperCase();
  const rgb = /^rgba?\(([^)]+)\)$/i.exec(value);
  if (rgb) {
    const channels = rgb[1]!.split(',').slice(0, 3).map(part => part.trim().endsWith('%') ? Number.parseFloat(part) * 2.55 : Number(part));
    if (channels.every(Number.isFinite)) return '#' + channels.map(channel => Math.max(0, Math.min(255, Math.round(channel))).toString(16).padStart(2, '0')).join('').toUpperCase();
  }
  return fallback;
}

function numberAttr(element: Element, name: string, fallback = 0) {
  const value = Number.parseFloat(element.getAttribute(name) || '');
  return Number.isFinite(value) ? value : fallback;
}

function shapeData(element: Element): Array<{ vertices: Vertex[]; closed: boolean }> {
  const tag = element.localName.toLowerCase();
  if (tag === 'path') return parseSvgPathData(element.getAttribute('d') || '');
  if (tag === 'polygon' || tag === 'polyline') {
    const values = (element.getAttribute('points') || '').match(NUMBER)?.map(Number) || [];
    const vertices: Vertex[] = [];
    for (let i = 0; i + 1 < values.length; i += 2) vertices.push(vertex(values[i]!, values[i + 1]!));
    return vertices.length ? [{ vertices, closed: tag === 'polygon' }] : [];
  }
  if (tag === 'line') return [{ vertices: [vertex(numberAttr(element, 'x1'), numberAttr(element, 'y1')), vertex(numberAttr(element, 'x2'), numberAttr(element, 'y2'))], closed: false }];
  if (tag === 'rect') {
    const x = numberAttr(element, 'x'), y = numberAttr(element, 'y'), w = numberAttr(element, 'width'), h = numberAttr(element, 'height');
    const rx = Math.min(w / 2, Math.max(0, numberAttr(element, 'rx', numberAttr(element, 'ry'))));
    const ry = Math.min(h / 2, Math.max(0, numberAttr(element, 'ry', rx)));
    if (!rx && !ry) return [{ vertices: [vertex(x, y), vertex(x + w, y), vertex(x + w, y + h), vertex(x, y + h)], closed: true }];
    return parseSvgPathData(`M${x + rx} ${y} H${x + w - rx} A${rx} ${ry} 0 0 1 ${x + w} ${y + ry} V${y + h - ry} A${rx} ${ry} 0 0 1 ${x + w - rx} ${y + h} H${x + rx} A${rx} ${ry} 0 0 1 ${x} ${y + h - ry} V${y + ry} A${rx} ${ry} 0 0 1 ${x + rx} ${y} Z`);
  }
  if (tag === 'circle' || tag === 'ellipse') {
    const cx = numberAttr(element, 'cx'), cy = numberAttr(element, 'cy');
    const rx = tag === 'circle' ? numberAttr(element, 'r') : numberAttr(element, 'rx');
    const ry = tag === 'circle' ? rx : numberAttr(element, 'ry');
    return parseSvgPathData(`M${cx + rx} ${cy} A${rx} ${ry} 0 1 1 ${cx - rx} ${cy} A${rx} ${ry} 0 1 1 ${cx + rx} ${cy} Z`);
  }
  return [];
}

function viewportMatrix(svg: Element, width: number, height: number): Matrix {
  const parts = (svg.getAttribute('viewBox') || '').match(NUMBER)?.map(Number);
  if (!parts || parts.length < 4 || !(parts[2]! > 0 && parts[3]! > 0)) return [1, 0, 0, 1, -width / 2, -height / 2];
  const [minX, minY, viewWidth, viewHeight] = parts;
  const preserve = (svg.getAttribute('preserveAspectRatio') || 'xMidYMid meet').trim();
  if (/^none(?:\s|$)/i.test(preserve)) return [width / viewWidth!, 0, 0, height / viewHeight!, -width / 2 - minX! * width / viewWidth!, -height / 2 - minY! * height / viewHeight!];
  const scale = /\bslice\b/i.test(preserve) ? Math.max(width / viewWidth!, height / viewHeight!) : Math.min(width / viewWidth!, height / viewHeight!);
  const renderedWidth = viewWidth! * scale, renderedHeight = viewHeight! * scale;
  let offsetX = 0, offsetY = 0;
  if (/xmid/i.test(preserve)) offsetX = (width - renderedWidth) / 2; else if (/xmax/i.test(preserve)) offsetX = width - renderedWidth;
  if (/ymid/i.test(preserve)) offsetY = (height - renderedHeight) / 2; else if (/ymax/i.test(preserve)) offsetY = height - renderedHeight;
  return [scale, 0, 0, scale, -width / 2 + offsetX - minX! * scale, -height / 2 + offsetY - minY! * scale];
}

export function parseSvg(source: string): ImportedSvg {
  const Parser = globalThis.DOMParser || (globalThis as any).window?.DOMParser;
  if (!Parser) throw new Error('SVG parsing is unavailable');
  const document = new Parser().parseFromString(source, 'image/svg+xml');
  if (document.querySelector('parsererror')) throw new Error('Could not parse this SVG file');
  const svg = document.documentElement;
  if (svg.localName.toLowerCase() !== 'svg') throw new Error('This file is not an SVG');
  const viewBox = (svg.getAttribute('viewBox') || '').match(NUMBER)?.map(Number);
  const rootLength = (name: string, fallback: number) => {
    const raw = svg.getAttribute(name) || '';
    return raw.includes('%') ? fallback : numberAttr(svg, name, fallback);
  };
  const width = rootLength('width', viewBox?.[2] || 0), height = rootLength('height', viewBox?.[3] || 0);
  if (!(width > 0 && height > 0)) throw new Error('SVG needs a positive width, height, or viewBox');
  const paths: ImportedSvgPath[] = [], warnings = new Set<string>();
  const supported = new Set(['path', 'rect', 'circle', 'ellipse', 'polygon', 'polyline', 'line']);
  const containers = new Set(['svg', 'g', 'a', 'switch']);
  type Style = { fill: string; fillOpacity: number; stroke: string; strokeWidth: number; strokeOpacity: number; opacity: number; display: string; visibility: string; fillRule: string };
  const walk = (element: Element, parentMatrix: Matrix, inherited: Style) => {
    const tag = element.localName.toLowerCase();
    if (['defs', 'clipPath', 'mask', 'pattern', 'linearGradient', 'radialGradient', 'metadata', 'title', 'desc'].map(v => v.toLowerCase()).includes(tag)) return;
    const inline = styleMap(element);
    const read = (name: string, fallback: string) => inline[name] ?? element.getAttribute(name) ?? fallback;
    const style: Style = {
      fill: read('fill', inherited.fill), stroke: read('stroke', inherited.stroke),
      fillOpacity: Math.max(0, Math.min(1, Number.parseFloat(read('fill-opacity', String(inherited.fillOpacity))))),
      strokeWidth: Number.parseFloat(read('stroke-width', String(inherited.strokeWidth))) || 0,
      strokeOpacity: Math.max(0, Math.min(1, Number.parseFloat(read('stroke-opacity', String(inherited.strokeOpacity))))),
      opacity: inherited.opacity * Math.max(0, Math.min(1, Number.parseFloat(read('opacity', '1')))),
      display: read('display', inherited.display), visibility: read('visibility', inherited.visibility),
      fillRule: read('fill-rule', inherited.fillRule),
    };
    if (style.display === 'none' || style.visibility === 'hidden') return;
    const matrix = multiply(parentMatrix, transformList(element.getAttribute('transform')));
    if (element.hasAttribute('class')) warnings.add('CSS class styles are not converted');
    if (supported.has(tag)) {
      let subpaths: Array<{ vertices: Vertex[]; closed: boolean }> = [];
      try { subpaths = shapeData(element); } catch (error) { warnings.add(error instanceof Error ? error.message : `Skipped invalid ${tag}`); }
      subpaths.forEach((subpath, index) => {
        const transformed = subpath.vertices.map(item => {
          const p = apply(matrix, item), incoming = apply(matrix, { x: item.x + item.inX, y: item.y + item.inY }), outgoing = apply(matrix, { x: item.x + item.outX, y: item.y + item.outY });
          return { x: p.x, y: p.y, inX: incoming.x - p.x, inY: incoming.y - p.y, outX: outgoing.x - p.x, outY: outgoing.y - p.y };
        });
        if (!transformed.length) return;
        if (/^url\(/i.test(style.fill) || /^url\(/i.test(style.stroke)) warnings.add('Gradient and pattern paints use a solid fallback');
        if (subpaths.length > 1 && style.fillRule.toLowerCase() === 'evenodd') warnings.add('Compound even-odd fills are imported as separate editable paths');
        const strokeScale = Math.sqrt(Math.abs(matrix[0] * matrix[3] - matrix[1] * matrix[2]));
        paths.push({
          name: element.getAttribute('id') || `${tag[0]!.toUpperCase()}${tag.slice(1)} ${paths.length + 1}${subpaths.length > 1 ? `.${index + 1}` : ''}`,
          vertices: transformed, closed: subpath.closed,
          fill: normalizedColor(style.fill, '#000000'), fillEnabled: subpath.closed && style.fill !== 'none',
          fillOpacity: style.fillOpacity * style.opacity * 100,
          stroke: normalizedColor(style.stroke, '#000000'), strokeWidth: style.stroke === 'none' ? 0 : Math.max(0, style.strokeWidth * strokeScale),
          strokeOpacity: style.strokeOpacity * style.opacity * 100,
        });
      });
    } else if (!containers.has(tag)) warnings.add(`Skipped unsupported <${tag}> content`);
    for (const child of Array.from(element.children)) walk(child, matrix, style);
  };
  walk(svg, viewportMatrix(svg, width, height), { fill: '#000000', fillOpacity: 1, stroke: 'none', strokeWidth: 1, strokeOpacity: 1, opacity: 1, display: 'inline', visibility: 'visible', fillRule: 'nonzero' });
  return { width, height, paths, warnings: [...warnings] };
}

export function materializeSvgPaths(PM: any, imported: ImportedSvg): VectorPath[] {
  return imported.paths.map(record => {
    const path = makeVectorPath(PM, record.name);
    Object.assign(path.p, {
      closed: PM.P(record.closed), fill: PM.P(record.fill), fillEnabled: PM.P(record.fillEnabled),
      fillOpacity: PM.P(record.fillOpacity), stroke: PM.P(record.stroke), strokeWidth: PM.P(record.strokeWidth), strokeOpacity: PM.P(record.strokeOpacity),
    });
    path.vertices = record.vertices.map(recordVertex => {
      const result = makeVertex(PM, recordVertex.x, recordVertex.y);
      for (const key of ['inX', 'inY', 'outX', 'outY'] as const) result.p[key].v = recordVertex[key];
      return result;
    });
    return path;
  });
}
