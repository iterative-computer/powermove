import type {
  Disposable,
  InspectorService as InspectorServiceContract,
  ServicesAPI,
  ShaderHooks as ShaderHooksContract,
  TimelineService as TimelineServiceContract,
  ToolService as ToolServiceContract,
  ViewerService as ViewerServiceContract
} from './api';

/** Runtime service contracts are declared in api.ts so the shipped API type
 * pack remains self-contained; these named interfaces are the kernel-side
 * registry vocabulary consumed by legacy adapters. */
export interface TimelineService extends TimelineServiceContract {}
export interface ViewerService extends ViewerServiceContract {}
export interface InspectorService extends InspectorServiceContract {}
export interface ToolService extends ToolServiceContract {}
export interface ShaderHooks extends ShaderHooksContract {}

export interface ServicesRegistry extends ServicesAPI {
  clear(): void;
  /** Kernel-side: `fn` runs whenever the implementation `name` resolves to changes. */
  onChange(name: string, fn: () => void): Disposable;
}

interface ServiceEntry {
  implementation: unknown;
  disposed: boolean;
}

/** A per-name stack: the latest live registration wins and disposing it
 * reveals the previous implementation, including out-of-order disposal. */
export function createServicesRegistry(): ServicesRegistry {
  const entries = new Map<string, ServiceEntry[]>();
  const listeners = new Map<string, Set<() => void>>();
  const top = (name: string): unknown => {
    const stack = entries.get(name);
    return stack?.[stack.length - 1]?.implementation;
  };
  /** Runs `change`, then the listeners of `name` if what it resolves to changed. */
  const changing = (name: string, change: () => void): void => {
    const before = top(name);
    change();
    if (top(name) === before) return;
    for (const fn of [...(listeners.get(name) ?? [])]) {
      try { fn(); } catch (error) { console.error(`[kernel] services.${name} listener failed`, error); }
    }
  };

  return {
    register<T>(name: string, implementation: T): Disposable {
      if (typeof name !== 'string' || !name) throw new Error('services.register requires a name');
      const entry: ServiceEntry = { implementation, disposed: false };
      changing(name, () => {
        const stack = entries.get(name) ?? [];
        stack.push(entry);
        entries.set(name, stack);
      });
      return {
        dispose(): void {
          if (entry.disposed) return;
          entry.disposed = true;
          changing(name, () => {
            const current = entries.get(name);
            if (!current) return;
            const index = current.indexOf(entry);
            if (index >= 0) current.splice(index, 1);
            if (!current.length) entries.delete(name);
          });
        }
      };
    },

    onChange(name: string, fn: () => void): Disposable {
      const set = listeners.get(name) ?? new Set();
      set.add(fn);
      listeners.set(name, set);
      return { dispose: () => void set.delete(fn) };
    },

    get<T>(name: string): T | null {
      const stack = entries.get(name);
      return (stack?.[stack.length - 1]?.implementation as T | undefined) ?? null;
    },

    clear(): void {
      const names = [...entries.keys()];
      for (const name of names) changing(name, () => entries.delete(name));
    }
  };
}
