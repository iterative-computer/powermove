// Rim-glow masks. Each glow layer is masked by a PNG drawn on a canvas: the
// body's measured box, rounded by its resolved radius, stroked with a conic
// gradient of OPACITY (not colour) and blurred. The blurred stroke is a soft
// band hugging the shape, which reads as light rather than a lit border.

export interface MaskStop {
  color: string;
  stop: number;
}

export interface MaskOptions {
  width: number;
  height: number;
  radius: number;
  strokeWidth: number;
  blur: number;
  alpha?: number;
  stops: MaskStop[];
  /** Fill the shape and punch a hole `ring` px inside it: the coloured border itself. */
  ring?: number;
}

/** Opaque over roughly a third of the circle, gone across the rest. This is
 *  where the partial coverage comes from; a full ring is a glowing border. */
export const RIM_STOPS: MaskStop[] = [
  { color: '#000', stop: 54 },
  { color: 'transparent', stop: 126 },
  { color: 'transparent', stop: 333 },
  { color: 'rgba(0,0,0,0.10)', stop: 347 },
  { color: '#000', stop: 360 },
];

export const RIM_LAYERS: { strokeWidth: number; blur: number; alpha: number; ring?: number }[] = [
  { strokeWidth: 0, blur: 0, alpha: 1, ring: 1 },
  { strokeWidth: 4, blur: 4, alpha: 0.3 },
  { strokeWidth: 8, blur: 8, alpha: 0.2 },
  { strokeWidth: 16, blur: 12, alpha: 0.1 },
  { strokeWidth: 20, blur: 20, alpha: 0.32 },
];

let scratch: HTMLCanvasElement | null = null;
const cache = new Map<string, string>();
const CACHE_MAX = 24;

const isSafari = typeof navigator !== 'undefined' && /^((?!chrome|android).)*safari/i.test(navigator.userAgent);

export function padOf(strokeWidth: number, blur: number): number {
  return Math.ceil(strokeWidth + blur * 3);
}

export function buildMask(o: MaskOptions): string {
  if (typeof document === 'undefined') return '';
  const key = [
    Math.round(o.width), Math.round(o.height), Math.round(o.radius),
    o.strokeWidth, o.blur, o.alpha ?? 1, o.ring ?? 0,
  ].join('|');
  const hit = cache.get(key);
  if (hit !== undefined) return hit;

  const pad = padOf(o.strokeWidth, o.blur);
  const w = Math.max(1, Math.ceil(o.width) + pad * 2);
  const h = Math.max(1, Math.ceil(o.height) + pad * 2);

  scratch ??= document.createElement('canvas');
  const c = scratch;
  c.width = w;
  c.height = h;
  const ctx = c.getContext('2d');
  if (!ctx) return '';
  ctx.clearRect(0, 0, w, h);

  // Safari's canvas blur is far stronger at the same radius.
  if (o.blur) ctx.filter = `blur(${isSafari ? o.blur * 0.25 : o.blur}px)`;

  const g = ctx.createConicGradient(0, w / 2, h / 2);
  for (const s of o.stops) g.addColorStop(s.stop / 360, s.color);
  ctx.strokeStyle = g;
  ctx.fillStyle = g;
  if (o.alpha != null) ctx.globalAlpha = o.alpha;

  const x = (w - o.width) / 2;
  const y = (h - o.height) / 2;
  const r = Math.max(0, Math.min(o.radius, o.width / 2, o.height / 2));
  ctx.beginPath();
  ctx.roundRect(x, y, o.width, o.height, r);

  if (o.strokeWidth) {
    ctx.lineWidth = o.strokeWidth;
    ctx.stroke();
  } else {
    ctx.fill();
    if (o.ring) {
      // Carve from a fill rather than stroking at width 1, or the line
      // straddles the edge and lands half a pixel outside the shape.
      ctx.globalCompositeOperation = 'destination-out';
      ctx.globalAlpha = 1;
      ctx.filter = 'none';
      const r2 = Math.max(0, Math.min(r - o.ring, (o.width - o.ring * 2) / 2, (o.height - o.ring * 2) / 2));
      ctx.beginPath();
      ctx.roundRect(x + o.ring, y + o.ring, Math.max(0, o.width - o.ring * 2), Math.max(0, o.height - o.ring * 2), r2);
      ctx.fill();
      ctx.globalCompositeOperation = 'source-over';
    }
  }

  const url = c.toDataURL('image/png');
  if (cache.size >= CACHE_MAX) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(key, url);
  return url;
}

export interface Layer {
  mask: string;
  pad: number;
}

export function buildLayers(width: number, height: number, radius: number): Layer[] {
  return RIM_LAYERS.map((l) => ({
    mask: buildMask({ width, height, radius, strokeWidth: l.strokeWidth, blur: l.blur, alpha: l.alpha, ring: l.ring, stops: RIM_STOPS }),
    pad: padOf(l.strokeWidth, l.blur),
  })).filter((l) => l.mask);
}

// The visible colour is a second conic under the mask. Each pulse rolls a new
// palette: a short bright arc of hues and a long dark tail.
const ARC_MIN = 90;
const ARC_MAX = 190;

function hsl(h: number, s: number, l: number, a = 1): string {
  const hue = ((h % 360) + 360) % 360;
  return a === 1 ? `hsl(${hue.toFixed(1)} ${s}% ${l}%)` : `hsl(${hue.toFixed(1)} ${s}% ${l}% / ${a})`;
}

export function rollPalette(el: HTMLElement, dark = false) {
  const anchor = Math.random() * 360;
  const arc = (ARC_MIN + Math.random() * (ARC_MAX - ARC_MIN)) * (Math.random() < 0.5 ? -1 : 1);
  const at = (t: number) => anchor + arc * t;
  const s = el.style;
  s.setProperty('--ai-c1', hsl(at(0), 96, 48));
  s.setProperty('--ai-c2', hsl(at(0.18), 52, 80));
  s.setProperty('--ai-c3', hsl(at(0.4), 98, 55));
  s.setProperty('--ai-c4', hsl(at(0.66), 96, 52));
  s.setProperty('--ai-c5', hsl(at(0.88), 94, 50));
  s.setProperty('--ai-c6', hsl(at(1), 90, 72, 0.8));
  s.setProperty('--ai-tail', hsl(at(-0.12), 70, 76, 0.63));
  if (dark) {
    s.setProperty('--ai-bg1', hsl(at(0), 18, 11));
    s.setProperty('--ai-bg2', hsl(at(0.5), 14, 8));
    s.setProperty('--ai-bg3', hsl(at(1), 16, 6));
  } else {
    s.setProperty('--ai-bg1', hsl(at(0), 62, 97));
    s.setProperty('--ai-bg2', hsl(at(0.5), 54, 95));
    s.setProperty('--ai-bg3', hsl(at(1), 58, 93));
  }
}
