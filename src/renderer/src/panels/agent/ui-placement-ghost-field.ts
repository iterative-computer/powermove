import { mount, unmount } from 'svelte';

/** The field is atmosphere, not detail, so it never renders at full Retina density. */
const MAX_DEVICE_PIXEL_RATIO = 1.5;

export type GhostFieldLoader = () => Promise<{ default: unknown }>;

/* WebGPU ghost code is optional and relatively heavy. Fetch it only once a run
   actually claims a panel, keeping normal editor startup lean. */
const loadGhostField: GhostFieldLoader = () => import('./GhostFieldCanvas.svelte');

/**
 * Mounts the Motion GPU generation field inside `host`, and reports through
 * `data-ghost-renderer` so the static CSS edge can take over whenever WebGPU is
 * unavailable, refused, or unwanted.
 */
export function mountGhostEdgeField(host: HTMLElement, load: GhostFieldLoader = loadGhostField): () => void {
  const motionQuery = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  let component: ReturnType<typeof mount> | null = null;
  let loading = false;
  let destroyed = false;

  const teardown = () => {
    if (!component) return;
    void unmount(component);
    component = null;
  };
  const fallback = (reason: string, error?: unknown) => {
    teardown();
    if (destroyed) return;
    host.dataset.ghostRenderer = 'static';
    host.dataset.ghostFallback = reason;
    if (error) window.console.warn('Motion GPU ghost field unavailable; using the static edge', error);
  };
  const sync = () => {
    if (destroyed) return;
    if (motionQuery?.matches) {
      fallback('reduced-motion');
      return;
    }
    if (!('gpu' in navigator)) {
      fallback('webgpu-unavailable');
      return;
    }
    if (component || loading) return;
    loading = true;
    host.dataset.ghostRenderer = 'motion-gpu-initializing';
    delete host.dataset.ghostFallback;
    void load().then(module => {
      loading = false;
      if (destroyed || motionQuery?.matches) return;
      component = mount(module.default as Parameters<typeof mount>[0], {
        target: host,
        props: {
          dpr: Math.min(window.devicePixelRatio || 1, MAX_DEVICE_PIXEL_RATIO),
          onError: (report: { rawMessage?: string; message?: string }) =>
            fallback('gpu-error', new Error(report?.rawMessage || report?.message || 'Motion GPU failed')),
          onFirstFrame: () => {
            if (destroyed) return;
            host.dataset.ghostRenderer = 'motion-gpu';
            delete host.dataset.ghostFallback;
          },
        },
      });
    }).catch(error => {
      loading = false;
      fallback('gpu-unavailable', error);
    });
  };

  motionQuery?.addEventListener?.('change', sync);
  sync();

  return () => {
    destroyed = true;
    motionQuery?.removeEventListener?.('change', sync);
    teardown();
  };
}
