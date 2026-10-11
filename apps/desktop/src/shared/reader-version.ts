/*
 * The `minReaderVersion` guard (docs/rust-engine.md, section 21).
 *
 * A writer that adds something older builds would lose or misread stamps the
 * document with the lowest reader version that understands it. A build whose
 * CURRENT_READER_VERSION is lower opens such a project read-only: it never
 * autosaves it and never saves over its file.
 *
 * The field lives at the project root (`proj.minReaderVersion`). The save
 * envelope root (`{ v, proj, history, ws }`) is honored too, so a writer may
 * stamp either place. A value that is present but not a finite number counts
 * as newer than every reader: an unknown stamp must not unlock editing.
 */

/** The document reader version this build implements. */
export const CURRENT_READER_VERSION = 1;

/** Shown when a project opens read-only because a newer build wrote it. */
export const NEWER_PROJECT_MESSAGE = 'This project needs a newer version of Powermove';

/** What an unreadable stamp is normalized to; JSON-safe, unlike Infinity. */
export const UNKNOWN_READER_VERSION = Number.MAX_SAFE_INTEGER;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value);

function stamp(holder: unknown): number {
  if (!isRecord(holder) || !Object.hasOwn(holder, 'minReaderVersion')) return 0;
  const value = holder.minReaderVersion;
  if (value === undefined || value === null) return 0;
  return typeof value === 'number' && Number.isFinite(value) ? value : UNKNOWN_READER_VERSION;
}

/**
 * The reader version a project file document or a bare project requires;
 * 0 when it carries no stamp.
 */
export function requiredReaderVersion(document: unknown): number {
  const project = isRecord(document) && isRecord(document.proj) ? document.proj : undefined;
  return Math.max(stamp(document), stamp(project));
}

/** True when this build must open the document read-only. */
export function needsNewerReader(document: unknown, reader: number = CURRENT_READER_VERSION): boolean {
  return requiredReaderVersion(document) > reader;
}
