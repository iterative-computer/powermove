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

/** Async document entry points wait for the native mirror before reading it. */
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
  throw new Error('pm-wasm-unavailable: The new engine document module did not become ready.');
}
