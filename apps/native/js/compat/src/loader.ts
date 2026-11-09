/** Records → load order → activate → health, after the reference's loader. An
 * extension that throws, hangs or keeps failing leaves the realm with one fewer
 * contributor and a recorded reason; it never stops the others. Module graphs
 * come from the native compiler and link with real ES semantics.
 */
import { linkRequest } from "./module-graph";
import type { CompatRealm } from "./realm";
import type { Data } from "./types";

const DEFAULT_TIMEOUT = 10_000;
const DEFAULT_WINDOW = 10_000;
const DEFAULT_LIMIT = 2;
const ERROR_CHARS = 4_000;
const BLOCKED = new Set([
  "build-error",
  "manifest-error",
  "needs-update",
  "needs-setup",
  "needs-trust",
]);

export function errorText(error: unknown): string {
  const raw =
    error instanceof Error ? error.message || String(error) : String(error);
  return raw.length > ERROR_CHARS ? `${raw.slice(0, ERROR_CHARS - 1)}…` : raw;
}
const withTimeout = <T>(
  promise: Promise<T>,
  ms: number,
  message: string,
): Promise<T> => {
  let timer: any;
  return Promise.race([
    promise,
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), ms);
    }),
  ]).finally(() => clearTimeout(timer));
};

export function planLoad(records: Data[]): {
  order: string[];
  skipped: { id: string; health: Data }[];
} {
  const skipped: { id: string; health: Data }[] = [];
  const byId = new Map(records.map((record) => [record.id, record]));
  const candidates = new Set<string>();
  for (const record of records) {
    if (record.enabled === false) {
      skipped.push({
        id: record.id,
        health:
          record.health?.state === "runtime-error"
            ? record.health
            : { state: "disabled" },
      });
      continue;
    }
    if (BLOCKED.has(record.health?.state)) continue;
    if (
      record.trust === "store" &&
      (record.manifest?.permissions ?? []).includes("full-access")
    ) {
      skipped.push({ id: record.id, health: { state: "needs-trust" } });
      continue;
    }
    candidates.add(record.id);
  }
  const replacedBy = new Map<string, string>();
  for (const id of candidates)
    for (const target of byId.get(id)?.manifest?.replaces ?? [])
      if (target !== id && candidates.has(target) && !replacedBy.has(target))
        replacedBy.set(target, id);
  for (const [target, by] of replacedBy) {
    candidates.delete(target);
    skipped.push({ id: target, health: { state: "replaced", by } });
  }
  for (let changed = true; changed; ) {
    changed = false;
    for (const id of [...candidates])
      for (const dep of byId.get(id)?.manifest?.dependsOn ?? []) {
        if (candidates.has(dep) || SHIPPED.has(dep)) continue;
        candidates.delete(id);
        skipped.push({
          id,
          health: { state: "activation-error", error: `requires ${dep}` },
        });
        changed = true;
        break;
      }
  }
  const pending = new Set([...candidates].sort());
  const order: string[] = [];
  while (pending.size) {
    const next = [...pending]
      .filter((id) =>
        (byId.get(id)?.manifest?.dependsOn ?? []).every(
          (dep: string) => !pending.has(dep),
        ),
      )
      .sort()[0];
    if (!next) break;
    pending.delete(next);
    order.push(next);
  }
  for (const id of pending)
    skipped.push({
      id,
      health: { state: "activation-error", error: "dependency cycle" },
    });
  return { order, skipped };
}

/** Built-ins the app provides natively, so they are never discovered records. */
const SHIPPED = new Set([
  "effects-basic",
  "inspector",
  "keymap-default",
  "layers-3d",
  "mods",
  "theme-default",
  "timeline",
  "toolbar",
  "transitions-basic",
  "viewer",
]);

export interface LoaderOptions {
  realm: CompatRealm;
  /** Binds a graph in V8 and returns `[evaluation, namespace]`. */
  link(graph: string): [Promise<unknown>, Data];
  /** Durable native health, Settings and toasts. Never re-enters this loader. */
  report(id: string, health: Data): void;
  /** The extension on the stack, so a host can name what hung. */
  activating?(id: string | null): void;
  toast?(text: string, options: Data): void;
  document?: any;
  vars?(record: Data): Data;
  timeoutMs?: number;
  windowMs?: number;
  limit?: number;
}

export function createLoader(options: LoaderOptions) {
  const { realm, link, report } = options;
  const kernel = realm.kernel;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT;
  const windowMs = options.windowMs ?? DEFAULT_WINDOW;
  const limit = options.limit ?? DEFAULT_LIMIT;
  const active = new Map<string, { record: Data; module: Data; handle: any }>();
  const records = new Map<string, Data>();
  const failures = new Map<string, number[]>();
  const failed = new Set<string>();
  const generations = new Map<string, number>();
  let queue: Promise<unknown> = Promise.resolve();
  let booting: Promise<void> | null = null;
  /** A new generation is a new module instance, like the reference's cache bust. */
  async function importGraph(record: Data): Promise<Data> {
    const graph = record.graph;
    if (!graph) throw new Error("no bundle to load");
    const generation = (generations.get(record.id) ?? 0) + 1;
    generations.set(record.id, generation);
    const [done, namespace] = link(
      linkRequest(
        graph,
        `pmext://${record.id}/${generation}/`,
        realm.globals.__powermove_runtime,
      ),
    );
    await done;
    return namespace;
  }
  const health = (id: string, value: Data) => {
    const record = records.get(id);
    if (record) record.health = value;
    report(id, value);
  };
  function failActivation(record: Data, error: unknown): void {
    const id = record.id;
    failed.add(id);
    kernel.disposeOwner(id);
    health(id, { state: "activation-error", error: errorText(error) });
    options.toast?.(`${record.manifest?.name ?? id} failed to load`, {
      sticky: true,
      source: { id, name: record.manifest?.name ?? id },
    });
  }
  async function activate(record: Data): Promise<boolean> {
    if (active.has(record.id)) return true;
    const id = record.id;
    let module: Data;
    try {
      module = await withTimeout(
        importGraph(record),
        timeoutMs,
        `import of "${id}" timed out`,
      );
      if (typeof module?.default !== "function")
        throw new Error("entry module must export default activate(api)");
    } catch (error) {
      failActivation(record, error);
      return false;
    }
    const handle: any = realm.extension(id, record.manifest?.apiVersion ?? 1, {
      manifest: record.manifest,
      scope: record.scope,
      vars: options.vars?.(record),
    });
    try {
      const addStyle = (css: string) => {
        if (!options.document) throw new Error("Remote DOM is not attached");
        const style = options.document.createElement("style");
        style.setAttribute("data-powermove-extension", id);
        style.textContent = css;
        handle.api.onDispose(() => style.remove());
        options.document.head.append(style);
      };
      if (module.__powermoveAcquireStyles)
        handle.api.onDispose(module.__powermoveAcquireStyles(addStyle));
      handle.setActivating(true);
      options.activating?.(id);
      const started = realm.task(() => module.default(handle.api));
      options.activating?.(null);
      const result = await withTimeout(
        Promise.resolve(started),
        timeoutMs,
        `activate() of "${id}" timed out`,
      );
      if (typeof result === "function") handle.api.onDispose(result);
      else if (typeof (result as Data)?.dispose === "function")
        handle.api.onDispose(() => (result as Data).dispose());
      realm.flush();
    } catch (error) {
      handle.dispose();
      failActivation(record, error);
      return false;
    } finally {
      handle.setActivating(false);
    }
    active.set(id, { record, module, handle });
    failed.delete(id);
    health(id, { state: "ok" });
    kernel.events.emit("extension:loaded", { id });
    return true;
  }
  async function deactivate(id: string): Promise<void> {
    const entry = active.get(id);
    active.delete(id);
    if (entry) {
      try {
        await withTimeout(
          Promise.resolve(entry.module?.deactivate?.()),
          timeoutMs,
          `deactivate() of "${id}" timed out`,
        );
      } catch (error) {
        console.error(`[kernel] extension "${id}" deactivate() threw`, error);
      }
      entry.handle.dispose();
    }
    kernel.disposeOwner(id);
    failures.delete(id);
    if (entry) kernel.events.emit("extension:unloaded", { id });
  }
  const enqueue = <T>(task: () => Promise<T>): Promise<T> => {
    const pending = queue.then(task);
    queue = pending.catch((error) =>
      console.error("[kernel] loader task failed", error),
    );
    return pending;
  };
  async function reconcile(
    list: Data[],
    changed: string[] = [],
  ): Promise<void> {
    for (const record of list) records.set(record.id, record);
    for (const id of changed) failed.delete(id);
    const plan = planLoad(list.filter((record) => !failed.has(record.id)));
    const desired = new Set(plan.order);
    const moved = new Set(changed);
    const panels = kernel.panels.batchChanges();
    try {
      for (const { id, health: value } of plan.skipped) health(id, value);
      for (const id of [...active.keys()].reverse())
        if (!desired.has(id) || moved.has(id)) await deactivate(id);
      for (const id of plan.order) {
        if (active.has(id)) continue;
        const record = records.get(id);
        if (!record) continue;
        // Each activation is its own task: the watchdog times one extension,
        // not a whole setup of them (and the page gets turns in between).
        await new Promise((resolve) => setTimeout(resolve, 0));
        await activate(record);
      }
    } finally {
      panels.dispose();
    }
  }
  function reportRuntimeError(id: string, error: unknown): void {
    if (!records.has(id)) return;
    const now = Date.now();
    const stamps = (failures.get(id) ?? []).filter((t) => now - t < windowMs);
    stamps.push(now);
    failures.set(id, stamps);
    if (stamps.length < limit) return;
    failures.delete(id);
    const name = records.get(id)?.manifest?.name ?? id;
    failed.add(id);
    void enqueue(async () => {
      const record = records.get(id);
      if (record) record.enabled = false;
      await deactivate(id);
      health(id, { state: "runtime-error", error: errorText(error) });
      options.toast?.(`${name} stopped working — check Mods`, {
        sticky: true,
        source: { id, name },
      });
    });
  }
  const loader = {
    /** No bundled v1–3 extensions: the rewritten built-ins are v4 realms. */
    builtinsReady: Promise.resolve(),
    boot(list: Data[]): Promise<void> {
      return (booting ??= enqueue(async () => {
        await reconcile(list);
        kernel.events.emit("extensions:ready", {});
      }));
    },
    update(list: Data[], ids: string[], reason: string): Promise<void> {
      return enqueue(async () => {
        await reconcile(list, ids);
        kernel.events.emit("extensions:changed", { ids: [...ids], reason });
      });
    },
    activate,
    deactivate,
    reload(id: string): Promise<void> {
      return enqueue(async () => {
        const panels = kernel.panels.batchChanges();
        try {
          await deactivate(id);
          failed.delete(id);
          const record = records.get(id);
          if (
            !record ||
            record.enabled === false ||
            BLOCKED.has(record.health?.state)
          )
            return;
          await activate(record);
        } finally {
          panels.dispose();
        }
      });
    },
    disable(id: string): Promise<void> {
      return enqueue(async () => {
        const record = records.get(id);
        if (record) record.enabled = false;
        await deactivate(id);
        health(id, { state: "disabled" });
      });
    },
    reportRuntimeError,
    records: () => [...records.values()],
    activeIds: () => [...active.keys()],
    whenIdle: () => queue.then(() => undefined),
    async dispose(): Promise<void> {
      for (const id of [...active.keys()].reverse()) await deactivate(id);
    },
  };
  return loader;
}
