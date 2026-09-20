// Profiling adapter for the production codec; never imported by the app.
import { HistoryRecords, packHistory, unpackHistory } from '../src/shared/history-memory.ts';

export function internHistory(history) {
  new HistoryRecords().share(history);
  const pools = new Map(), counts = {};
  for (const entry of history.entries) for (const direction of ['forward', 'backward']) {
    for (const patch of entry[direction]) {
      if (patch.path?.length !== 1 || !['layers', 'edits'].includes(patch.path[0]) || !Array.isArray(patch.value)) continue;
      const category = patch.path[0];
      if (!pools.has(category)) pools.set(category, new Set());
      const pool = pools.get(category);
      const count = counts[category] ??= { references: 0, unique: 0, repeatedJsonBytes: 0, uniqueJsonBytes: 0 };
      for (const value of patch.value) {
        const bytes = Buffer.byteLength(JSON.stringify(value));
        count.references++; count.repeatedJsonBytes += bytes;
        if (!pool.has(value)) { pool.add(value); count.unique++; count.uniqueJsonBytes += bytes; }
      }
    }
  }
  return counts;
}
export const encodeHistory = history => JSON.stringify(packHistory(history));
export const decodeHistory = encoded => unpackHistory(JSON.parse(encoded));
