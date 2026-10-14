/*
 * The production write barrier for writers outside history (docs/rust-engine.md,
 * section 6, "Making every write visible"): `writeDocument(PM, paths, fn)` runs
 * `fn` as a known writer of `paths` (relative to `PM.proj`, as writers see it).
 * Registries without the engine-doc install (unit fixtures) just run `fn`.
 */
import type { PathPart } from '../../../shared/patch';

export function writeDocument<T>(PM: any, paths: readonly PathPart[][], fn: () => T): T {
  const doc = PM?.doc;
  return typeof doc?.write === 'function' ? doc.write(paths, fn) as T : fn();
}

/** Records paths already written (relative to `PM.proj`). */
export function touchedDocument(PM: any, paths: readonly PathPart[][]): void {
  const doc = PM?.doc;
  if (typeof doc?.touched === 'function') doc.touched(paths);
}
