import type { PMRegistry } from '../legacy/registry';
import type { EngineDoc } from './install';

/**
 * Whether an async document entry point must wait before reading the
 * document: the new engine is on and its module is not ready yet. Callers
 * await `waitForDocumentReady` only then, so a ready (or disabled) engine adds
 * no microtask to a request that must stay in its original tick.
 */
export function documentPending(PM: PMRegistry): boolean {
  const doc: EngineDoc | undefined = PM.EngineDoc;
  return !!doc?.enabled && !doc.ready;
}

/**
 * Async document entry points wait for the native mirror before reading it.
 * A load that fails (strict mode) rejects with the named failure and shows the
 * engine's failure notice again, so a request is never dropped silently.
 */
export async function waitForDocumentReady(PM: PMRegistry): Promise<void> {
  const doc: EngineDoc | undefined = PM.EngineDoc;
  if (!doc?.enabled || doc.ready) return;
  // The boot load, or a Retry or revival that is still in flight.
  const ready = await (doc.settled?.() ?? doc.whenReady);
  if (ready && doc.ready) return;
  // Outside strict mode the module's own fallback answers (development modes).
  if (doc.strict === false) return;
  // The load failed (or a formerly ready instance died). This is a real named
  // engine failure; it must not turn the request into a legacy evaluation.
  doc.showFailure?.();
  throw new Error('pm-wasm-unavailable: The new engine document module did not become ready.');
}

/**
 * Built-in editor commands that never wait for the document module. Most open
 * app surfaces (Settings, where "Use New Engine" is switched off, the palette,
 * the agent, Projects) or windows and tabs, or start opening another project
 * (whose own open path waits for the document module). Save writes the
 * JavaScript project, which needs no module (its consistency check is skipped
 * until the module is ready), so a failed load can still be saved by shortcut
 * and menu as it can from the toolbar.
 *
 * The field-editing commands (`blurField` and the `context*` Edit commands)
 * act on the focused text field or text selection when one owns the key:
 * native paste, copy, undo and so on, which never touch the document. They
 * run at once too, and gate only their document fallback (`pasteLayers`,
 * `copyLayers`, `undo`, ...) with `runWhenDocumentReady`.
 *
 * These ids count as app commands only while the built-in definition is
 * active. Every other command, including any extension command and an
 * extension's override of one of these ids, is treated as a document command
 * and waits; that is the safe default (commands carry no metadata saying
 * whether they read the document).
 */
export const APP_COMMANDS: ReadonlySet<string> = new Set([
  'settings', 'palette', 'agent', 'projects',
  'open', 'newProject', 'save', 'saveAs',
  'closeTab', 'nextTab', 'previousTab', 'moveTabToNewWindow',
  'blurField', 'contextUndo', 'contextRedo', 'contextCut', 'contextCopy', 'contextPaste', 'contextSelectAll',
]);

/** The owner the legacy layer registers its built-in commands under (legacy/ui/shortcuts.ts). */
const BUILT_IN_OWNER = 'legacy';

/**
 * Whether a command reads or writes the document (see `APP_COMMANDS`). With a
 * registry, an app command id that an extension has overridden is a document
 * command.
 */
export function documentCommand(id: string, PM?: PMRegistry): boolean {
  if (!APP_COMMANDS.has(id)) return true;
  const owner = (PM as { Kernel?: { commands?: { topEntry?(id: string): { ownerId: string } | undefined } } } | undefined)
    ?.Kernel?.commands?.topEntry?.(id)?.ownerId;
  return owner !== undefined && owner !== BUILT_IN_OWNER;
}

/**
 * Runs work that reads or writes the document. It runs now when the document
 * can answer; while the module is loading (boot, Retry or revival) it runs
 * once that settles, and if the load fails it does not run and the named
 * failure is shown instead. Returns the work's result, or undefined when it
 * was queued.
 */
export function runWhenDocumentReady(PM: PMRegistry, label: string, run: () => unknown): unknown {
  if (!documentPending(PM)) return run();
  const failed = (error: unknown) => { console.error(`[engine-doc] command ${label} failed`, error); };
  void waitForDocumentReady(PM).then(
    () => {
      let result: unknown;
      try { result = run(); } catch (error) { failed(error); return; }
      if (result && typeof (result as PromiseLike<unknown>).then === 'function') {
        Promise.resolve(result).catch(failed);
      }
    },
    () => {}, // waitForDocumentReady showed the named failure.
  );
  return undefined;
}

/**
 * Runs a command from a key or the native menu. An app command runs now; a
 * document command goes through `runWhenDocumentReady`.
 */
export function runCommandWhenReady(PM: PMRegistry, id: string, run: () => unknown): unknown {
  if (!documentCommand(id, PM)) return run();
  return runWhenDocumentReady(PM, id, run);
}
