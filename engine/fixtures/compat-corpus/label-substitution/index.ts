import type { PowermoveAPI } from 'powermove';

export default function activate(api: PowermoveAPI): void {
  const previous = api.services.get<(...args: any[]) => any>('raster')!;
  const registration = api.services.register('raster', (layer: any, scale = 1, time = 0, uploaded?: any, crop?: any) => {
    if (layer.id !== 'dynamic-label') return previous(layer, scale, time, uploaded, crop);
    const text = time < 1 ? 'I' : 'Wide label';
    const changed = { ...layer, id: 'detached-label', d: { ...api.anim.resolveContent(layer, time), text } };
    const surface = previous(changed, scale, time, undefined, crop);
    return surface && { ...surface, key: `label-substitution:${text}:${scale}` };
  });
  api.onDispose(() => registration.dispose());
}
