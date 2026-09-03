import { mount, unmount } from 'svelte';

export type GpuFieldLoader = () => Promise<{ default: unknown }>;

export interface GpuFieldOptions {
  /** Dataset key set to the live renderer state, e.g. `ghostRenderer`. */
  rendererKey: string;
  /** Dataset key set to the reason the static treatment took over. */
  fallbackKey: string;
  /** Names the field in the console when a GPU failure is reported. */
  label: string;
  /** Atmosphere never renders at full Retina density. */
  maxDevicePixelRatio: number;
}

/**
 * Mounts a Motion GPU field inside `host` and reports its renderer through the
 * host's dataset, so the static CSS treatment can take over whenever WebGPU is
 * unavailable, refused, or unwanted.
 */
export function mountGpuField(host: HTMLElement, load: GpuFieldLoader, options: GpuFieldOptions): () => void {
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
    host.dataset[options.rendererKey] = 'static';
    host.dataset[options.fallbackKey] = reason;
    if (error) window.console.warn(`${options.label} unavailable; using the static edge`, error);
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
    host.dataset[options.rendererKey] = 'motion-gpu-initializing';
    delete host.dataset[options.fallbackKey];
    void load().then(module => {
      loading = false;
      if (destroyed || motionQuery?.matches) return;
      component = mount(module.default as Parameters<typeof mount>[0], {
        target: host,
        props: {
          dpr: Math.min(window.devicePixelRatio || 1, options.maxDevicePixelRatio),
          onError: (report: { rawMessage?: string; message?: string }) =>
            fallback('gpu-error', new Error(report?.rawMessage || report?.message || 'Motion GPU failed')),
          onFirstFrame: () => {
            if (destroyed) return;
            host.dataset[options.rendererKey] = 'motion-gpu';
            delete host.dataset[options.fallbackKey];
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
