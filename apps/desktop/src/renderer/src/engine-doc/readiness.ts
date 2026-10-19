import type { PMRegistry } from '../legacy/registry';
import type { EngineDoc } from './install';

/** Async document entry points wait for the native mirror before reading it. */
export async function waitForDocumentReady(PM: PMRegistry): Promise<void> {
  const doc: EngineDoc | undefined = PM.EngineDoc;
  if (!doc?.enabled || doc.ready) return;
  if (await doc.whenReady && doc.ready) return;
  // The load failed (or a formerly ready instance died). This is a real named
  // engine failure; it must not turn the request into a legacy evaluation.
  throw new Error('pm-wasm-unavailable: The new engine document module did not become ready.');
}
