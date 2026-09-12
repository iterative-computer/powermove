/*
 * Live views over the kernel registries, shaped like the legacy plain objects
 * they replace (`PM.PANELS`, `PM.commands`, `PM.FX`, `PM.TRANSITIONS`).
 *
 * The point of the migration is that the kernel becomes the single store. But
 * dozens of legacy call sites read those members as objects — `PM.PANELS[id]`,
 * `Object.values(PM.commands)`, `'blur' in PM.FX`, and (overlays/install.ts)
 * `PM.commands.palette.run = fn`. A Proxy keeps every one of those spellings
 * working while the data lives in exactly one place.
 *
 * Symbol keys fall through to the backing object so Svelte/Vitest internals
 * (`Symbol.toStringTag`, `$state` markers, inspection) behave normally.
 */
import { createKernel, type Kernel } from '../kernel/registries';
import type { Registry } from '../kernel/registry';

/** The kernel PM already has, or a bare one so legacy installs work standalone (tests). */
export function ensureKernel(PM: Record<string, any>): Kernel {
  const existing = PM.Kernel as Kernel | undefined;
  if (existing?.panels && existing?.commands) return existing;
  const kernel = createKernel();
  PM.Kernel = kernel;
  return kernel;
}

export interface RegistryViewOptions<T extends { id: string }, V> {
  /** Map a stored registry item to the legacy shape callers expect. */
  read(item: T, id: string): V;
  /** Handle `view[id] = value`. Return false to reject the assignment. */
  write?(id: string, value: unknown): boolean | void;
  /** Handle `delete view[id]`. */
  remove?(id: string): boolean | void;
}

export function registryView<T extends { id: string }, V>(
  registry: Registry<T>,
  options: RegistryViewOptions<T, V>
): Record<string, V> {
  const value = (id: string): V | undefined => {
    const item = registry.get(id);
    return item === undefined ? undefined : options.read(item, id);
  };
  return new Proxy({} as Record<string, V>, {
    get(target, key, receiver) {
      if (typeof key === 'symbol') return Reflect.get(target, key);
      return registry.has(key) ? value(key) : Reflect.get(target, key, receiver);
    },
    set(target, key, next) {
      if (typeof key === 'symbol') return Reflect.set(target, key, next);
      if (!options.write) return false;
      return options.write(key, next) !== false;
    },
    deleteProperty(target, key) {
      if (typeof key === 'symbol') return Reflect.deleteProperty(target, key);
      const entry = registry.topEntry(key);
      if (!entry) return Reflect.deleteProperty(target, key);

      /* Delete the active definition, regardless of who registered it. Calling
         an owner-specific legacy handle here can leave an extension override
         in place (or remove the buried legacy fallback instead). Removing the
         concrete top entry gives the facade ordinary-object delete semantics
         while preserving a stacked fallback definition, when one exists. */
      registry.disposeEntry(entry);

      /* The legacy views keep their own handles so assignments can replace an
         earlier value. Once the id is completely gone, let them discard that
         handle too. Do not call this hook when a fallback is now active. */
      if (!registry.has(key)) options.remove?.(key);
      return true;
    },
    has(target, key) {
      if (typeof key === 'symbol') return Reflect.has(target, key);
      return registry.has(key) || Reflect.has(target, key);
    },
    ownKeys: (target) => [...new Set([...Reflect.ownKeys(target), ...registry.ids()])],
    getOwnPropertyDescriptor(target, key) {
      if (typeof key === 'symbol') return Reflect.getOwnPropertyDescriptor(target, key);
      if (!registry.has(key)) return Reflect.getOwnPropertyDescriptor(target, key);
      // `configurable: true` is required: the target has no such own property,
      // and a non-configurable report would violate the Proxy invariants.
      return { value: value(key), enumerable: true, configurable: true, writable: true };
    }
  });
}
