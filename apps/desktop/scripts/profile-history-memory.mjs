// Read-only investigation: node --expose-gc scripts/profile-history-memory.mjs /path/to/projectHistory.ID.json
// Shares identical immutable layer/edit records in memory, verifies that the
// complete serialized history is unchanged, and reports post-GC V8 heap usage.
// Exercises the production interner and versioned dictionary codec.
import { readFileSync } from 'node:fs';
import { basename } from 'node:path';
import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import { internHistory, encodeHistory, decodeHistory } from './history-memory-prototype.mjs';

if (!global.gc || process.argv.length !== 3) {
  throw new Error('Usage: node --expose-gc profile-history-memory.mjs /path/to/history.json');
}
const digest = value => createHash('sha256').update(value).digest('hex');
const heap = () => {
  for (let i = 0; i < 3; i++) global.gc();
  return process.memoryUsage().heapUsed;
};
const baseline = heap();
const started = performance.now();
function loadHistory(path) {
  const history = JSON.parse(readFileSync(path, 'utf8'));
  if (!Array.isArray(history.entries)) throw new Error('Expected a saved project history');
  const serialized = JSON.stringify(history);
  return { history, originalHash: digest(serialized), originalJsonBytes: Buffer.byteLength(serialized) };
}
// Keep serialization temporaries off the measurement's live stack. Explicit GC
// alone cannot collect a temporary still retained in an interpreter register.
const { history, originalHash, originalJsonBytes } = loadHistory(process.argv[2]);
await new Promise(resolve => setImmediate(resolve));
const parsed = heap();
const parseMs = performance.now() - started;

const internStarted = performance.now();
const counts = internHistory(history);
const internMs = performance.now() - internStarted;
await new Promise(resolve => setImmediate(resolve));
const interned = heap();
const roundTripExact = digest(JSON.stringify(history)) === originalHash;
if (!roundTripExact) throw new Error('History serialization changed');

// A proposed disk representation must also avoid expanding the duplicates on
// every relaunch. Encode shared records once and verify decode through JSON.
const encoded = encodeHistory(history);
const restored = decodeHistory(encoded);
const diskRoundTripExact = digest(JSON.stringify(restored)) === originalHash;
if (!diskRoundTripExact) throw new Error('Dictionary codec changed history');
console.log(JSON.stringify({
  file: basename(process.argv[2]), node: process.version,
  entries: history.entries.length, index: history.index, roundTripExact,
  heapBytes: { baseline, parsed, interned, parsedIncrement: parsed - baseline, internedIncrement: interned - baseline },
  reductionPercent: 100 * (parsed - interned) / (parsed - baseline),
  diskExperiment: { originalJsonBytes, dictionaryJsonBytes: Buffer.byteLength(encoded),
    dictionaryGzipBytes: gzipSync(encoded).byteLength, diskRoundTripExact },
  parseAndGcMs: parseMs, experimentalInternMs: internMs, counts,
}, null, 2));
