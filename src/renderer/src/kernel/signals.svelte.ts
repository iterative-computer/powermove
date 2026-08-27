/*
 * Reactive change signals for the kernel registries.
 *
 * Registries are plain (non-reactive) data structures on purpose: the kernel
 * must work in a plain Node test with no Svelte runtime. Svelte surfaces that
 * need to re-render when a registry changes read the matching counter here and
 * `installKernelSignals` bumps it from each registry's `onChange`.
 *
 * `$state` on a number keeps the invalidation coarse but exact: one bump per
 * registry mutation, no proxying of the registry contents themselves.
 */
import type { Disposable } from './api';
import type { Kernel } from './registries';

class KernelSignals {
  panels = $state(0);
  commands = $state(0);
  effects = $state(0);
  transitions = $state(0);
  themes = $state(0);
  status = $state(0);
  menus = $state(0);
}

export const kernelSignals = new KernelSignals();

/** Bump every counter — used by tests and by a full kernel teardown. */
export function resetKernelSignals(): void {
  kernelSignals.panels = 0;
  kernelSignals.commands = 0;
  kernelSignals.effects = 0;
  kernelSignals.transitions = 0;
  kernelSignals.themes = 0;
  kernelSignals.status = 0;
  kernelSignals.menus = 0;
}

/** Subscribe the signal counters to a kernel's registries. */
export function installKernelSignals(kernel: Kernel): Disposable {
  const subs: Disposable[] = [
    kernel.panels.onChange(() => void kernelSignals.panels++),
    kernel.commands.onChange(() => void kernelSignals.commands++),
    kernel.effects.onChange(() => void kernelSignals.effects++),
    kernel.transitions.onChange(() => void kernelSignals.transitions++),
    kernel.themes.onChange(() => void kernelSignals.themes++),
    kernel.status.onChange(() => void kernelSignals.status++)
  ];
  /* Menu contributions are a plain list, not a Registry, so the bump is wired
     by wrapping the one mutating entry point. */
  const contributeMenu = kernel.contributeMenu.bind(kernel);
  const patched = kernel as { contributeMenu: Kernel['contributeMenu'] };
  patched.contributeMenu = (ownerId, location, items) => {
    const disposable = contributeMenu(ownerId, location, items);
    kernelSignals.menus++;
    return {
      dispose: () => {
        disposable.dispose();
        kernelSignals.menus++;
      }
    };
  };
  return {
    dispose: () => {
      patched.contributeMenu = contributeMenu;
      subs.forEach((sub) => sub.dispose());
    }
  };
}
