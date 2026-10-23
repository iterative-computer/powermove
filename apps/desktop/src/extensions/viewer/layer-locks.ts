import type { Layer, PowermoveAPI } from 'powermove';

export function layerIsLocked(api: PowermoveAPI, layer: Layer): boolean {
  return !!layer.lock || (api.groups.ancestors(layer) || []).some(group => group.lock);
}
