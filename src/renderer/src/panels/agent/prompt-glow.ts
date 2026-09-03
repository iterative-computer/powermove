import { mountGpuField, type GpuFieldLoader } from './gpu-field';

/** The halo is atmosphere, not detail, so it never renders at full Retina density. */
const MAX_DEVICE_PIXEL_RATIO = 1.5;

/* WebGPU code is optional and relatively heavy. Fetch it only once a run
   actually starts, keeping normal editor startup lean. */
const loadPromptGlow: GpuFieldLoader = () => import('./PromptGlowCanvas.svelte');

/**
 * Mounts the Motion GPU halo around the prompt being answered, and reports
 * through `data-glow-renderer` so the static CSS ring can take over whenever
 * WebGPU is unavailable, refused, or unwanted.
 */
export function mountPromptGlow(host: HTMLElement, load: GpuFieldLoader = loadPromptGlow): () => void {
  return mountGpuField(host, load, {
    rendererKey: 'glowRenderer',
    fallbackKey: 'glowFallback',
    label: 'Motion GPU prompt glow',
    maxDevicePixelRatio: MAX_DEVICE_PIXEL_RATIO,
  });
}
