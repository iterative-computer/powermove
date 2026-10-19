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
 * Editor commands that never read or write the document: they open app
 * surfaces (Settings, where "Use New Engine" is switched off, the palette,
 * the agent, Projects) or windows and tabs, or start opening another project
 * (whose own open path waits for the document module). These run at once from
 * a key or the native menu even while the module is loading or after it
 * failed. Every other command, including any extension command, is treated as
 * a document command and waits; that is the safe default.
 */
export const APP_COMMANDS: ReadonlySet<string> = new Set([
  'settings', 'palette', 'agent', 'projects',
  'open', 'newProject',
  'closeTab', 'nextTab', 'previousTab', 'moveTabToNewWindow',
]);

/** Whether a command reads or writes the document (see `APP_COMMANDS`). */
export function documentCommand(id: string): boolean {
  return !APP_COMMANDS.has(id);
}

/**
 * Runs a command from a key or the native menu. An app command runs now. A
 * document command runs now when the document can answer; while the module
 * is loading (boot, Retry or revival) it runs once that settles, and if the
 * load fails it does not run and the named failure is shown instead.
 * Returns the command's result, or undefined when it was queued.
 */
export function runCommandWhenReady(PM: PMRegistry, id: string, run: () => unknown): unknown {
  if (!documentCommand(id) || !documentPending(PM)) return run();
  void waitForDocumentReady(PM).then(
    () => { try { run(); } catch (error) { console.error(`[engine-doc] command ${id} failed`, error); } },
    () => {}, // waitForDocumentReady showed the named failure.
  );
  return undefined;
}
