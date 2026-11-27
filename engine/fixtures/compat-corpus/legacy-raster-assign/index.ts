import type { PowermoveAPI } from 'powermove';

// The pattern of user extensions written before the services registry
// (an extension can shift anchors this way): capture
// PM.raster, assign a wrapper, restore it on dispose.
export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as any;
  const raster = PM.raster;
  const wrapped = (layer: any, scale = 1, time = PM.time, uploaded?: unknown, crop?: unknown) => {
    const result = raster(layer, scale, time, uploaded, crop);
    if (!result) return result;
    if (layer?.type === 'text' && layer.name === 'Shifted') {
      return { ...result, anchorX: result.anchorX + 24 };
    }
    if (layer?.type === 'shape' && layer.name === 'Tinted' && result.cv) {
      const canvas = document.createElement('canvas');
      canvas.width = result.cv.width;
      canvas.height = result.cv.height;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(result.cv, 0, 0);
      ctx.globalCompositeOperation = 'source-in';
      ctx.fillStyle = '#E8202A';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      return { ...result, cv: canvas, key: `legacy-tint:${layer.id}:${scale}:${api.anim.version()}` };
    }
    return result;
  };
  PM.raster = wrapped;
  api.onDispose(() => {
    if (PM.raster === wrapped) PM.raster = raster;
    api.transport.invalidate();
  });
  api.transport.invalidate();
}
