/**
 * The generated sweep of Canvas2D raster cases for pm-vector's compat level 1
 * rasterizer (docs/engine/vector.md). Deterministic: the same list every run.
 *
 * A case is either a shape (`rasterShape(d, density)`) or a layer's editable
 * paths (`rasterPaths(PM, layer, time, scale)`), with static content values.
 */
export type ShapeCase = { id: string; kind: 'shape'; d: Record<string, unknown>; scale: number };
export type PathSpec = {
  id: string; parent: string | null;
  p: Record<string, unknown>;
  vertices: Array<{ x: number; y: number; inX?: number; inY?: number; outX?: number; outY?: number }>;
};
export type PathsCase = { id: string; kind: 'paths'; paths: PathSpec[]; scale: number };
/** Canvas calls replayed as they are: [method, ...args] or ['=prop', value]. */
export type RawCase = { id: string; kind: 'raw'; width: number; height: number; ops: Array<Array<string | number>> };
/** An SVG drawn as `rasterSvgAsset` draws it: drawImage(img, 0, 0, w, h). */
export type SvgCase = { id: string; kind: 'svg'; svg: string; width: number; height: number };
export type Case = ShapeCase | PathsCase | RawCase | SvgCase;

/** A small deterministic generator (mulberry32). */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// `continuousRasterScale` returns quarter steps between 0.25 and 32.
const SCALES = [0.25, 0.5, 1, 1.25, 2, 3.75];
const COLORS = ['#FF6B1A', '#4C8DFF', '#3FCF8E80', '#E8E2CF', '#00000000', '#FFFFFF'];

function shapeBase(shape: string): Record<string, unknown> {
  return { shape, w: 120, h: 90, color: '#FF6B1A', radius: 0, stroke: 0, strokeColor: '#ffffff', points: 5 };
}

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (id: string, d: Record<string, unknown>, scale: number) =>
    out.push({ id: `shape-${id}-s${scale}`, kind: 'shape', d, scale });

  // Every shape type at every scale, filled and filled plus stroked.
  for (const shape of ['rect', 'ellipse', 'polygon', 'star', 'line']) {
    for (const scale of SCALES) {
      add(`${shape}-fill`, { ...shapeBase(shape), h: shape === 'line' ? 0 : 90 }, scale);
      add(`${shape}-stroke`, { ...shapeBase(shape), h: shape === 'line' ? 10 : 90, stroke: 6, strokeColor: '#4C8DFFCC' }, scale);
    }
  }
  // One property at a time, at 1x and a fractional scale.
  const variants: Array<[string, Record<string, unknown>]> = [
    ['rect-radius', { ...shapeBase('rect'), radius: 20 }],
    ['rect-radius-huge', { ...shapeBase('rect'), radius: 400 }],
    ['rect-radius-frac', { ...shapeBase('rect'), w: 101.3, h: 47.7, radius: 9.6 }],
    ['rect-radius-stroke', { ...shapeBase('rect'), radius: 14, stroke: 4, strokeColor: '#3FCF8E' }],
    ['rect-corners', { ...shapeBase('rect'), independentCorners: true, radiusTL: 0, radiusTR: 30, radiusBR: 8, radiusBL: 45 }],
    ['rect-corners-null', { ...shapeBase('rect'), radius: 12, independentCorners: true, radiusTL: null, radiusTR: 30 }],
    ['rect-corners-stroke', { ...shapeBase('rect'), independentCorners: true, radiusTL: 4, radiusTR: 30, radiusBR: 0, radiusBL: 18, stroke: 5, strokeColor: '#FFFFFF' }],
    ['rect-smooth-60', { ...shapeBase('rect'), radius: 30, smoothing: 60 }],
    ['rect-smooth-100', { ...shapeBase('rect'), radius: 30, smoothing: 100 }],
    ['rect-smooth-corners', { ...shapeBase('rect'), independentCorners: true, radiusTL: 10, radiusTR: 40, radiusBR: 0, radiusBL: 25, smoothing: 80, stroke: 3 }],
    ['rect-frac', { ...shapeBase('rect'), w: 37.5, h: 61.25 }],
    ['rect-thin', { ...shapeBase('rect'), w: 3, h: 200 }],
    ['rect-tiny', { ...shapeBase('rect'), w: 0.6, h: 0.6 }],
    ['rect-stroke-thin', { ...shapeBase('rect'), stroke: 0.5, strokeColor: '#FFFFFF' }],
    ['rect-stroke-thick', { ...shapeBase('rect'), stroke: 30, strokeColor: '#4C8DFF80' }],
    ['rect-stroke-only', { ...shapeBase('rect'), radius: 14, color: '#00000000', stroke: 4, strokeColor: '#3FCF8E' }],
    ['rect-color-alpha', { ...shapeBase('rect'), color: '#3FCF8E80' }],
    ['rect-color-invalid', { ...shapeBase('rect'), color: 'not-a-color' }],
    ['rect-stroke-color-empty', { ...shapeBase('rect'), stroke: 3, strokeColor: '' }],
    ['rect-stroke-color-named', { ...shapeBase('rect'), stroke: 3, strokeColor: 'rebeccapurple' }],
    ['ellipse-frac', { ...shapeBase('ellipse'), w: 77.3, h: 41.9 }],
    ['ellipse-circle', { ...shapeBase('ellipse'), w: 64, h: 64 }],
    ['ellipse-stroke-white', { ...shapeBase('ellipse'), w: 120, h: 80, stroke: 6, strokeColor: '#FFFFFF' }],
    ['ellipse-thin', { ...shapeBase('ellipse'), w: 200, h: 4 }],
    ['ellipse-big', { ...shapeBase('ellipse'), w: 700, h: 500 }],
    ['polygon-3', { ...shapeBase('polygon'), points: 3 }],
    ['polygon-6', { ...shapeBase('polygon'), w: 110, h: 110, points: 6 }],
    ['polygon-frac-points', { ...shapeBase('polygon'), points: 5.5 }],
    ['polygon-24-stroke', { ...shapeBase('polygon'), points: 24, stroke: 3, strokeColor: '#FFFFFF' }],
    ['polygon-big', { ...shapeBase('polygon'), w: 640, h: 640, points: 7 }],
    ['star-7', { ...shapeBase('star'), w: 120, h: 120, points: 7, color: '#E8E2CF', stroke: 3, strokeColor: '#FF6B1A' }],
    ['star-3', { ...shapeBase('star'), points: 3 }],
    ['star-24', { ...shapeBase('star'), points: 24, w: 200, h: 200 }],
    ['star-thick-stroke', { ...shapeBase('star'), stroke: 14, strokeColor: '#4C8DFF' }],
    ['star-big', { ...shapeBase('star'), w: 800, h: 700 }],
    ['line-thin', { ...shapeBase('line'), w: 140, h: 0, stroke: 0.5 }],
    ['line-default', { ...shapeBase('line'), w: 140, h: 0, stroke: 0 }],
    ['line-thick', { ...shapeBase('line'), w: 300, h: 40, stroke: 25, color: '#3FCF8E80' }],
    ['line-frac', { ...shapeBase('line'), w: 99.7, h: 3.3, stroke: 8.4 }],
  ];
  for (const [id, d] of variants) for (const scale of [1, 1.25, 2.75]) add(id, d, scale);

  // A seeded random sweep across every property together.
  const r = rng(0x5eed);
  const pick = <T,>(xs: T[]) => xs[Math.floor(r() * xs.length)]!;
  for (let i = 0; i < 160; i++) {
    const shape = pick(['rect', 'rect', 'ellipse', 'polygon', 'star', 'line']);
    const d: Record<string, unknown> = {
      shape,
      w: Math.round((4 + r() * 300) * 4) / 4,
      h: shape === 'line' ? Math.round(r() * 30) : Math.round((4 + r() * 300) * 4) / 4,
      color: pick(COLORS),
      radius: r() < 0.5 ? 0 : Math.round(r() * 60 * 2) / 2,
      stroke: r() < 0.4 ? 0 : Math.round(r() * 16 * 4) / 4,
      strokeColor: pick(COLORS),
      points: 3 + Math.floor(r() * 10),
    };
    if (shape === 'rect' && r() < 0.35) {
      Object.assign(d, { independentCorners: true, radiusTL: Math.round(r() * 50), radiusTR: Math.round(r() * 50), radiusBR: Math.round(r() * 50), radiusBL: Math.round(r() * 50) });
    }
    if (shape === 'rect' && r() < 0.3) d.smoothing = Math.round(r() * 100);
    add(`random${i}`, d, pick(SCALES));
  }

  // Editable paths.
  const P = (p: Record<string, unknown>) => ({
    x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, closed: false, fill: '#E8E2CF', fillEnabled: true, fillOpacity: 100,
    stroke: '#ffffff', strokeWidth: 2, strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0, ...p,
  });
  const wave = [{ x: -200, y: 0, outX: 60, outY: -90 }, { x: 0, y: 0, inX: -60, inY: 90, outX: 60, outY: -90 }, { x: 200, y: 0, inX: -60, inY: 90 }];
  const badge = [{ x: -60, y: -40, outX: 30, outY: 0 }, { x: 60, y: -40, inX: -30, inY: 0 }, { x: 60, y: 40 }, { x: -60, y: 40 }];
  const petal = [{ x: 0, y: 0, outX: 20, outY: -30 }, { x: 0, y: -70, inX: 20, inY: 0, outX: -20, outY: 0 }];
  const tri = [{ x: 0, y: 0 }, { x: 6, y: 0 }, { x: 6, y: 6 }];
  const pathCases: Array<[string, PathSpec[]]> = [
    ['wave-stroke', [{ id: 'a', parent: null, p: P({ fillEnabled: false, stroke: '#4C8DFF', strokeWidth: 8 }), vertices: wave }]],
    ['wave-trim', [{ id: 'a', parent: null, p: P({ fillEnabled: false, stroke: '#4C8DFF', strokeWidth: 8, trimEnd: 60, trimOffset: 45 }), vertices: wave }]],
    ['wave-trim-wrap', [{ id: 'a', parent: null, p: P({ fillEnabled: false, strokeWidth: 5, trimStart: 20, trimEnd: 70, trimOffset: 300 }), vertices: wave }]],
    ['badge', [{ id: 'b', parent: null, p: P({ closed: true, fill: '#FF6B1A', fillOpacity: 80, stroke: '#FFFFFF', strokeWidth: 3, strokeOpacity: 60 }), vertices: badge }]],
    ['badge-nostroke', [{ id: 'b', parent: null, p: P({ closed: true, fill: '#FF6B1A', strokeWidth: 0 }), vertices: badge }]],
    ['badge-trimmed-fill', [{ id: 'b', parent: null, p: P({ closed: true, fill: '#FF6B1A', trimEnd: 50 }), vertices: badge }]],
    ['badge-rotated-scaled', [{ id: 'b', parent: null, p: P({ closed: true, fill: '#1B2A4A', rotation: 33, scaleX: 140, scaleY: 70, x: 12.5, y: -7.25 }), vertices: badge }]],
    ['parented', [
      { id: 'b', parent: null, p: P({ closed: true, fill: '#FF6B1A', x: -50, y: 30, rotation: 15 }), vertices: badge },
      { id: 'c', parent: 'b', p: P({ closed: true, fill: '#1B2A4A', rotation: 45, strokeWidth: 0 }), vertices: [{ x: -20, y: -20 }, { x: 20, y: -20 }, { x: 20, y: 20 }, { x: -20, y: 20 }] },
    ]],
    ['repeater', [{ id: 'r', parent: null, p: P({ closed: true, fill: '#3FCF8E', copies: 12, repeatX: 0, repeatY: 0, repeatRotation: 30, stroke: '#0C0D12', strokeWidth: 1 }), vertices: petal }]],
    ['repeater-256', [{ id: 'm', parent: null, p: P({ closed: true, fill: '#F2C14E', copies: 300, repeatX: 2, repeatY: 0.4, repeatRotation: 1.4, strokeWidth: 0 }), vertices: tri }]],
    ['open-fill-ignored', [{ id: 'o', parent: null, p: P({ closed: false, fill: '#FF0000', strokeWidth: 3 }), vertices: badge }]],
    ['invalid-colors', [{ id: 'i', parent: null, p: P({ closed: true, fill: 'bogus', stroke: 'nope', strokeWidth: 4 }), vertices: badge }]],
    ['hairline', [{ id: 'h', parent: null, p: P({ fillEnabled: false, strokeWidth: 0.4 }), vertices: wave }]],
    ['compound', [
      { id: 'o', parent: null, p: P({ closed: true, fill: '#4C8DFF', strokeWidth: 0 }), vertices: [{ x: -50, y: -50 }, { x: 50, y: -50 }, { x: 50, y: 50 }, { x: -50, y: 50 }] },
      { id: 'i', parent: null, p: P({ closed: true, fill: '#FFFFFF', fillOpacity: 90, strokeWidth: 0 }), vertices: [{ x: -30, y: 10, outX: 10, outY: -40 }, { x: 30, y: 10, inX: -10, inY: -40 }, { x: 0, y: 35 }] },
    ]],
    ['self-intersecting', [{ id: 's', parent: null, p: P({ closed: true, fill: '#A73366', strokeWidth: 2 }), vertices: [{ x: -60, y: -40 }, { x: 60, y: 40 }, { x: 60, y: -40 }, { x: -60, y: 40 }] }]],
  ];
  for (const [id, paths] of pathCases) for (const scale of [0.5, 1, 1.75, 3]) out.push({ id: `paths-${id}-s${scale}`, kind: 'paths', paths, scale });
  out.push(...diagnostics(), ...svgCases());
  return out;
}

/** Raw canvas diagnostics for the emulation's open questions. */
function diagnostics(): RawCase[] {
  const out: RawCase[] = [];
  // Rotated triangles with degenerate cubic edges (as tracePath writes
  // corners) and with lines, at several scales.
  for (const curve of [true, false]) {
    for (let k = 0; k < 24; k++) {
      const deg = k * 7.3 + 1.1, scale = [0.5, 1.5, 3][k % 3]!;
      const ops: Array<Array<string | number>> = [['scale', scale, scale], ['translate', 12 / scale, 12 / scale], ['rotate', deg * Math.PI / 180], ['beginPath'], ['moveTo', 0, 0]];
      const pts = [[6, 0], [6, 6], [0, 0]];
      let prev = [0, 0];
      for (const p of pts) {
        if (curve) ops.push(['bezierCurveTo', prev[0]!, prev[1]!, p[0]!, p[1]!, p[0]!, p[1]!]);
        else ops.push(['lineTo', p[0]!, p[1]!]);
        prev = p;
      }
      ops.push(['closePath'], ['=fillStyle', '#F2C14E'], ['fill']);
      out.push({ id: `diag-tri-${curve ? 'cubic' : 'line'}-${k}`, kind: 'raw', width: 40, height: 40, ops });
    }
  }
  // Large ellipses around the conic chopping threshold.
  for (const r of [200, 260, 320, 380, 440, 520, 640]) {
    const ops: Array<Array<string | number>> = [['beginPath'], ['ellipse', r + 3.3, r * 0.7 + 2.7, r, r * 0.7, 0, 0, 7], ['=fillStyle', '#FF6B1A'], ['fill']];
    out.push({ id: `diag-ellipse-${r}`, kind: 'raw', width: Math.ceil(2 * r + 8), height: Math.ceil(1.4 * r + 8), ops });
  }
  return out;
}

/** SVG images (`<img>` per size bucket). */
function svgCases(): SvgCase[] {
  const svgs: Array<[string, string]> = [
    ['solid', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M10 10 L90 20 L60 90 Z" fill="#4C8DFF"/><circle cx="70" cy="35" r="18" fill="#FF6B1A"/></svg>'],
    ['stroke', '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80"><rect x="10.5" y="10.5" width="100" height="60" rx="12" fill="none" stroke="#3FCF8E" stroke-width="5"/><polyline points="20,60 50,25 80,55 105,20" fill="none" stroke="#E8E2CF" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/></svg>'],
    ['opacity', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><g opacity="0.6"><rect x="4" y="4" width="40" height="40" fill="#A73366"/><rect x="20" y="20" width="40" height="40" fill="#F2C14E"/></g><ellipse cx="32" cy="50" rx="20" ry="8" fill="#4C8DFF" fill-opacity="0.5"/></svg>'],
    ['gradient', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#FF6B1A"/><stop offset="1" stop-color="#4C8DFF"/></linearGradient><radialGradient id="r"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient></defs><rect width="100" height="60" rx="8" fill="url(#g)"/><circle cx="70" cy="30" r="25" fill="url(#r)"/></svg>'],
    ['clip', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 80"><defs><clipPath id="c"><circle cx="40" cy="40" r="30"/></clipPath></defs><g clip-path="url(#c)"><rect width="80" height="40" fill="#3FCF8E"/><rect y="40" width="80" height="40" fill="#1B2A4A"/></g></svg>'],
    ['evenodd', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill-rule="evenodd" d="M50 5 L61 39 L97 39 L68 61 L79 95 L50 74 L21 95 L32 61 L3 39 L39 39 Z M50 30 L58 52 L42 52 Z" fill="#E8E2CF"/></svg>'],
    ['blur', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 80"><defs><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="4"/></filter></defs><rect x="20" y="20" width="40" height="40" fill="#FF6B1A" filter="url(#b)"/></svg>'],
    ['text', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 50"><text x="10" y="35" font-family="Helvetica" font-size="28" fill="#4C8DFF">Powermove</text></svg>'],
    ['transform', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><g transform="rotate(20 50 50) scale(0.9)"><rect x="20" y="20" width="60" height="40" fill="#3FCF8E" stroke="#0C0D12" stroke-width="3" stroke-dasharray="8 4"/></g></svg>'],
  ];
  const sizes: Array<[number, number]> = [[64, 64], [200, 200], [333, 150]];
  return svgs.flatMap(([id, svg]) => sizes.map(([w, h]) => ({ id: `svg-${id}-${w}x${h}`, kind: 'svg' as const, svg, width: w, height: h })));
}
