// Extracted from v1.2.0 kernel/services.ts; registry behavior unchanged.
import type { Disposable, ServicesAPI } from "./types";
export interface ServicesRegistry extends ServicesAPI {
  clear(): void;
}

interface ServiceEntry {
  implementation: unknown;
  disposed: boolean;
}

/** A per-name stack: the latest live registration wins and disposing it
 * reveals the previous implementation, including out-of-order disposal. */
export function createServicesRegistry(): ServicesRegistry {
  const entries = new Map<string, ServiceEntry[]>();

  return {
    register<T>(name: string, implementation: T): Disposable {
      if (typeof name !== "string" || !name)
        throw new Error("services.register requires a name");
      const stack = entries.get(name) ?? [];
      const entry: ServiceEntry = { implementation, disposed: false };
      stack.push(entry);
      entries.set(name, stack);
      return {
        dispose(): void {
          if (entry.disposed) return;
          entry.disposed = true;
          const current = entries.get(name);
          if (!current) return;
          const index = current.indexOf(entry);
          if (index >= 0) current.splice(index, 1);
          if (!current.length) entries.delete(name);
        },
      };
    },

    get<T>(name: string): T | null {
      const stack = entries.get(name);
      return (
        (stack?.[stack.length - 1]?.implementation as T | undefined) ?? null
      );
    },

    clear(): void {
      entries.clear();
    },
  };
}
