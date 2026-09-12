import { mountGpuField, type GpuFieldLoader } from './gpu-field';

/** The field is atmosphere, not detail, so it never renders at full Retina density. */
const MAX_DEVICE_PIXEL_RATIO = 1.5;

export type GhostFieldLoader = GpuFieldLoader;

/* WebGPU ghost code is optional and relatively heavy. Fetch it only once a run
   actually claims a panel, keeping normal editor startup lean. */
const loadGhostField: GhostFieldLoader = () => import('./GhostFieldCanvas.svelte');

/**
 * Mounts the Motion GPU generation field inside `host`, and reports through
 * `data-ghost-renderer` so the static CSS edge can take over whenever WebGPU is
 * unavailable, refused, or unwanted.
 */
export function mountGhostEdgeField(host: HTMLElement, load: GhostFieldLoader = loadGhostField): () => void {
  return mountGpuField(host, load, {
    rendererKey: 'ghostRenderer',
    fallbackKey: 'ghostFallback',
    label: 'Motion GPU ghost field',
    maxDevicePixelRatio: MAX_DEVICE_PIXEL_RATIO,
  });
}
