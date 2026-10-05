import type { PowermoveAPI } from 'powermove';
import { animatorMode, countTextUnits, staggerWindow } from 'powermove';

/* Stagger text animators have no keyframes, so the clip shows each one as a
   cascade: the first unit's interval along the top edge, the last unit's
   along the bottom. Dragging it moves the animator's Start; dragging its end
   stretches Duration and Stagger together. */

export interface StaggerBand {
  animator: any;
  values: { delay: number; duration: number; stagger: number };
  /** Seconds from the layer's in point. */
  start: number;
  duration: number;
  cascade: number;
  end: number;
}

export type StaggerHit = { band: StaggerBand; part: 'body' | 'end' };

const TIMING = ['delay', 'duration', 'stagger'] as const;

export function createTextStagger(api: PowermoveAPI) {
  const counts = new WeakMap<object, { version: number; units: Record<string, number> }>();

  function units(layer: any, unit: 'characters' | 'words' | 'lines'): number {
    const version = api.anim.version();
    let entry = counts.get(layer);
    if (!entry || entry.version !== version) counts.set(layer, entry = { version, units: {} });
    return entry.units[unit] ??= Math.max(1, countTextUnits(api.render.textLayout(layer, api.transport.time())?.lines ?? [], unit));
  }

  function bands(layer: any): StaggerBand[] {
    if (layer?.type !== 'text' || !layer.d?.animators?.length) return [];
    const time = api.transport.time();
    return layer.d.animators
      .filter((animator: any) => animator.enabled !== false && animatorMode(animator) === 'stagger' && animator.p?.duration)
      .map((animator: any) => {
        const values = Object.fromEntries(TIMING.map((key) => [key,
          animator.p[key] ? Number(api.anim.evP(layer, animator.p[key], time, `ta.${animator.id}.${key}`)) || 0 : 0])) as StaggerBand['values'];
        return { animator, values, ...staggerWindow(values, units(layer, animator.unit ?? 'characters'), animator.order) };
      });
  }

  /** Geometry inside a clip whose body spans `top`…`top + height`. */
  function geometry(band: StaggerBand, layer: any, t2x: (time: number) => number, top: number, height: number) {
    const y1 = top + height - 2, y0 = y1 - Math.min(8, Math.max(4, height * 0.3));
    const at = (seconds: number) => t2x(layer.from + seconds);
    return {
      y0, y1,
      points: [[at(band.start), y0], [at(band.start + band.duration), y0], [at(band.end), y1], [at(band.start + band.cascade), y1]] as Array<[number, number]>,
      x0: at(band.start), x1: at(band.end)
    };
  }

  function draw(c: CanvasRenderingContext2D, layer: any, t2x: (time: number) => number, top: number, height: number, color: string) {
    for (const band of bands(layer)) {
      const { points } = geometry(band, layer, t2x, top, height);
      c.save();
      c.globalAlpha *= 0.8;
      c.fillStyle = color;
      c.beginPath();
      points.forEach(([x, y], index) => index ? c.lineTo(x, y) : c.moveTo(x, y));
      c.closePath();
      c.fill();
      c.restore();
    }
  }

  /** The lower half of the clip is the band's hit area, so it stays easy to grab. */
  function hit(layer: any, x: number, y: number, t2x: (time: number) => number, top: number, height: number): StaggerHit | null {
    if (y < top + height / 2 || y > top + height) return null;
    for (const band of bands(layer).reverse()) {
      const { x0, x1 } = geometry(band, layer, t2x, top, height);
      if (Math.abs(x - x1) < 5) return { band, part: 'end' };
      if (x >= x0 - 2 && x <= x1) return { band, part: 'body' };
    }
    return null;
  }

  /** Timing channel values for a drag of `dt` seconds. */
  function drag(band: StaggerBand, part: StaggerHit['part'], dt: number, fps: number): Partial<StaggerBand['values']> {
    const frame = (seconds: number) => Math.round(seconds * fps) / fps;
    if (part === 'body') return { delay: frame(band.values.delay + dt) };
    const span = band.duration + band.cascade;
    if (span <= 0) return { duration: Math.max(0, frame(dt)) };
    const scale = Math.max(1 / fps, frame(span + dt)) / span;
    const round = (value: number) => Math.round(value * 1000) / 1000;
    return { duration: round(band.values.duration * scale), stagger: round(band.values.stagger * scale) };
  }

  return { bands, draw, hit, drag };
}
