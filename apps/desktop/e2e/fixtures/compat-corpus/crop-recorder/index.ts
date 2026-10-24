import type { PowermoveAPI } from 'powermove';

type Raster = (layer: any, scale?: number, time?: number, uploaded?: unknown, crop?: unknown) => any;

// A pass-through wrapper that records what the compositor asks of the
// raster service: each layer's density and crop window.
export default function activate(api: PowermoveAPI): void {
  const w = window as any;
  w.__crops = [] as Array<{ layer: string; scale: number; time?: number; crop: unknown }>;
  api.events.on('extensions:ready', () => install());
  const install = () => {
    if (w.__cropRecorder) return;
    const original = api.services.get<Raster>('raster');
    if (typeof original !== 'function') return;
    w.__cropRecorder = api.services.register<Raster>('raster', (layer, scale = 1, time, uploaded, crop) => {
      if (w.__crops.length < 4000) w.__crops.push({ layer: String(layer?.id ?? ''), scale, time, crop: crop ? JSON.parse(JSON.stringify(crop)) : null });
      return original(layer, scale, time, uploaded, crop);
    });
  };
  w.__installCropRecorder = install;
}
