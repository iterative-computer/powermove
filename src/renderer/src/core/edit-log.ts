// The edit log is provenance, separate from the history patches used by Undo.
// Bulk edits can contain entire rotoscoping tracks or composition snapshots.
// Retain their descriptions without copying those payloads on every later edit.
export const MAX_EDIT_LOG_ENTRIES = 200;
export const MAX_EDIT_LOG_ENTRY_BYTES = 128 * 1024;
export const MAX_EDIT_LOG_PAYLOAD_BYTES = 1024 * 1024;

type Entry = { operations?: unknown; structural?: unknown; payloadOmitted?: boolean };
const sizes = new WeakMap<object, number>();
const encoder = new TextEncoder();

function payloadBytes(entry: Entry): number {
  const cached = sizes.get(entry);
  if (cached !== undefined) return cached;
  let bytes: number;
  try {
    bytes = encoder.encode(JSON.stringify({ operations: entry.operations, structural: entry.structural })).byteLength;
  } catch {
    bytes = Infinity;
  }
  sizes.set(entry, bytes);
  return bytes;
}

/** Entries are immutable after recording; newest normal-sized payloads win. */
export function compactEditLog<T extends Entry>(entries: readonly T[]): T[] {
  let remaining = MAX_EDIT_LOG_PAYLOAD_BYTES;
  return entries.filter(entry => entry && typeof entry === 'object').slice(-MAX_EDIT_LOG_ENTRIES).reverse().map(entry => {
    const bytes = payloadBytes(entry);
    if (bytes <= MAX_EDIT_LOG_ENTRY_BYTES && bytes <= remaining) {
      remaining -= bytes;
      return entry;
    }
    const { structural: _structural, ...metadata } = entry;
    return { ...metadata, operations: [], payloadOmitted: true } as T;
  }).reverse();
}
