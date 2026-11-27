// Development-only entry; excluded from the production runtime bundle.
import "@prefresh/core";
import { flushUpdates } from "./renderer";
interface Runtime {
  register(type: unknown, id: string): void;
  sign(
    type: unknown,
    key: string,
    reset: boolean,
    hooks: () => unknown[],
    status: string,
  ): string;
  getPendingUpdates(): [unknown, unknown][];
  flush(): void;
  replaceComponent(previous: unknown, next: unknown, reset: boolean): void;
}
const signatures = new Map<
  unknown,
  { key: string; reset: boolean; hooks: () => unknown[] }
>();
function runtime(): Runtime {
  return (globalThis as unknown as { __PREFRESH__: Runtime }).__PREFRESH__;
}
export function register(type: unknown, stableModuleAndExportId: string) {
  runtime().register(type, stableModuleAndExportId);
}
export function signature(
  type: unknown,
  key: string,
  reset = false,
  hooks: () => unknown[] = () => [],
) {
  signatures.set(type, { key, reset, hooks });
  runtime().sign(type, key, reset, hooks, "begin");
  runtime().sign(type, key, reset, hooks, "needsHooks");
}
function compatible(
  previous: unknown,
  next: unknown,
  seen = new Set<unknown>(),
): boolean {
  const a = signatures.get(previous),
    b = signatures.get(next);
  if (!a && !b) return true; // Best-effort uninstrumented development updates.
  if (!a || !b || a.key !== b.key || b.reset) return false;
  if (seen.has(previous)) return false;
  seen.add(previous);
  const before = a.hooks(),
    after = b.hooks();
  return (
    before.length === after.length &&
    before.every(
      (hook, i) =>
        hook === after[i] ||
        (signatures.has(hook) &&
          signatures.has(after[i]) &&
          compatible(hook, after[i], new Set(seen))),
    )
  );
}
/** Compiler supplies stable IDs and hook signatures; incompatible signatures reset state. */
export function refresh(
  updates: { previous: unknown; next: unknown; reset?: boolean }[] = [],
) {
  const pending = runtime().getPendingUpdates().slice();
  runtime().flush();
  for (const [previous, next] of pending) {
    const incompatible = !compatible(previous, next);
    runtime().replaceComponent(
      previous,
      next,
      incompatible ||
        !!updates.find((u) => u.previous === previous && u.next === next)
          ?.reset,
    );
  }
  flushUpdates();
}
