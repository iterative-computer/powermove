/*
 * Shared runtime for extension bundles.
 *
 * Main compiles user/project extensions with `external: ['svelte', 'svelte/*']`,
 * so their bundles resolve those specifiers at load time against
 * `globalThis.__powermove_runtime`. Importing the namespaces statically here is
 * what makes that safe: Vite bundles exactly ONE copy of the Svelte runtime into
 * the app, and every extension component shares the host's reactivity graph
 * (two copies would mean effects that never run and stores that never notify).
 */
import * as svelte from 'svelte';
/* `svelte/internal/client` ships no declaration file. It is imported purely so
   Vite bundles the one copy extension bundles resolve against — nothing here
   calls into it, so the missing types cost us nothing. */
// @ts-expect-error -- untyped Svelte internal entrypoint
import * as svelteInternalClient from 'svelte/internal/client';
import * as svelteStore from 'svelte/store';
import type { Component } from 'svelte';

export interface PowermoveRuntime {
  svelte: typeof svelte;
  'svelte/internal/client': typeof svelteInternalClient;
  'svelte/store': typeof svelteStore;
  'svelte/internal/disclose-version': Record<string, never>;
}

declare global {
  // eslint-disable-next-line no-var
  var __powermove_runtime: PowermoveRuntime | undefined;
}

export const RUNTIME_GLOBAL = '__powermove_runtime' as const;

export function installRuntimeGlobals(): PowermoveRuntime {
  const runtime: PowermoveRuntime = {
    svelte,
    'svelte/internal/client': svelteInternalClient,
    'svelte/store': svelteStore,
    'svelte/internal/disclose-version': {}
  };
  globalThis.__powermove_runtime = runtime;
  return runtime;
}

export function runtimeGlobals(): PowermoveRuntime | undefined {
  return globalThis.__powermove_runtime;
}

/**
 * Mount a Svelte component and return its disposer. Effects are flushed
 * synchronously so callers (panel `build`, overlays) can read the DOM straight
 * after mounting — the same contract `registerSveltePanel` relies on.
 */
export function mountComponent<P extends Record<string, unknown>>(component: Component<P>, target: HTMLElement, props: P): () => void {
  const instance = svelte.mount(component, { target, props });
  svelte.flushSync();
  let unmounted = false;
  return () => {
    if (unmounted) return;
    unmounted = true;
    void svelte.unmount(instance);
  };
}
