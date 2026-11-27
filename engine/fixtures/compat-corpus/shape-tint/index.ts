import type { Layer, PowermoveAPI, RasterSurface } from 'powermove';

type Surface = RasterSurface & { cv: HTMLCanvasElement; w: number; h: number; anchorX: number; anchorY: number; key?: string };
type Raster = (layer: Layer, scale?: number, time?: number, uploaded?: unknown, crop?: unknown) => Surface | null;

// An apiVersion 1 raster override, the way extensions wrote them before the
// engine: capture the service, pass unaffected layers through untouched,
// return a new canvas for the layers it changes.
export default function activate(api: PowermoveAPI): void {
  const original = api.services.get<Raster>('raster');
  if (typeof original !== 'function') throw new Error('No raster service.');
  api.services.register<Raster>('raster', (layer, scale = 1, time = api.transport.time(), uploaded, crop) => {
    const base = original(layer, scale, time, uploaded, crop);
    if (layer.type !== 'shape' || layer.name !== 'Tinted' || !base?.cv) return base;
    const canvas = document.createElement('canvas');
    canvas.width = base.cv.width;
    canvas.height = base.cv.height;
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(base.cv, 0, 0);
    ctx.globalCompositeOperation = 'source-in';
    ctx.fillStyle = '#E8202A';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    // A time-dependent mark: a bar whose width follows the time.
    ctx.globalCompositeOperation = 'source-over';
    ctx.fillStyle = '#FFFFFF';
    ctx.fillRect(0, 0, Math.round(canvas.width * Math.min(1, time / 3)), Math.max(1, Math.round(4 * scale)));
    return { ...base, cv: canvas, key: `shape-tint:${layer.id}:${scale}:${time}:${api.anim.version()}` };
  });
  api.transport.invalidate();
}
