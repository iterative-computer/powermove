import { appendFileSync, statfsSync } from 'node:fs';
import { rm, writeFile } from 'node:fs/promises';

/** Stream snapshots to disk; an event callback must never throw into CDP. */
export async function heapSnapshot(cdp, file, {
  reserveBytes = 5_500_000_000,
  freeBytes = () => { const s = statfsSync('/'); return s.bavail * s.bsize; }
} = {}) {
  let bytes = 0, failure;
  const chunk = ({ chunk }) => {
    if (failure) return;
    try {
      const size = Buffer.byteLength(chunk);
      if (freeBytes() <= reserveBytes + size) throw new Error('Heap snapshot would exhaust disk reserve');
      appendFileSync(file, chunk);
      bytes += size;
    } catch (error) { failure = error; }
  };
  await writeFile(file, '');
  cdp.on('HeapProfiler.addHeapSnapshotChunk', chunk);
  try {
    await cdp.send('HeapProfiler.takeHeapSnapshot', { reportProgress: false });
    if (failure) throw failure;
    return bytes;
  } catch (error) {
    await rm(file, { force: true });
    throw error;
  } finally { cdp.off('HeapProfiler.addHeapSnapshotChunk', chunk); }
}
