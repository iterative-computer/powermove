import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { heapSnapshot } from './studio-editor-heap.mjs';

it('streams a complete heap snapshot and removes its listener', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'studio-heap-test-'));
  const file = path.join(directory, 'heap.json');
  const cdp = Object.assign(new EventEmitter(), { async send() {
    this.emit('HeapProfiler.addHeapSnapshotChunk', { chunk: '{"value":' });
    this.emit('HeapProfiler.addHeapSnapshotChunk', { chunk: '"é"}' });
  } });
  try {
    expect(await heapSnapshot(cdp, file, { freeBytes: () => 100, reserveBytes: 0 })).toBe(14);
    expect(JSON.parse(await readFile(file, 'utf8'))).toEqual({ value: 'é' });
    expect(cdp.listenerCount('HeapProfiler.addHeapSnapshotChunk')).toBe(0);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

it.each(['disk', 'transport'])('cleans an interrupted %s snapshot without throwing from its event callback', async reason => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'studio-heap-test-'));
  const file = path.join(directory, 'heap.json');
  const cdp = Object.assign(new EventEmitter(), { async send() {
    this.emit('HeapProfiler.addHeapSnapshotChunk', { chunk: '{' });
    // CDP can continue emitting after space is exhausted. Ignore those chunks.
    this.emit('HeapProfiler.addHeapSnapshotChunk', { chunk: '}' });
    if (reason === 'transport') throw new Error('Session closed');
  } });
  try {
    await expect(heapSnapshot(cdp, file, { freeBytes: () => reason === 'disk' ? 0 : 100, reserveBytes: 0 }))
      .rejects.toThrow(reason === 'disk' ? 'disk reserve' : 'Session closed');
    await expect(stat(file)).rejects.toMatchObject({ code: 'ENOENT' });
    expect(cdp.listenerCount('HeapProfiler.addHeapSnapshotChunk')).toBe(0);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
