import { describe, expect, it, vi } from 'vitest';
import { writeBlobChunks } from './blob-chunks';

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function controlledBlob(size = 10) {
  const reads: Array<ReturnType<typeof deferred<ArrayBuffer>> & { start: number; end: number; complete(): void }> = [];
  let active = 0, peak = 0;
  const blob = { size, slice: (start: number, end: number) => ({ arrayBuffer: () => {
    const read = deferred<ArrayBuffer>();
    reads.push({ ...read, start, end, complete: () => read.resolve(Uint8Array.from({ length: Math.min(end, size) - start }, (_, index) => start + index).buffer) });
    active++; peak = Math.max(peak, active);
    return read.promise.finally(() => { active--; });
  } }) } as Blob;
  return { blob, reads, active: () => active, peak: () => peak };
}

describe('bounded Blob delivery', () => {
  it('prepares only the next read while the current write is pending and preserves every byte', async () => {
    const source = controlledBlob();
    const writes: Array<{ bytes: Uint8Array; offset: number; done: ReturnType<typeof deferred<void>> }> = [];
    let activeWrites = 0, peakWrites = 0;
    const sending = writeBlobChunks(source.blob, async (bytes, offset) => {
      activeWrites++; peakWrites = Math.max(peakWrites, activeWrites);
      const done = deferred<void>(); writes.push({ bytes, offset, done });
      try { await done.promise; } finally { activeWrites--; }
    }, 4);
    expect(source.reads.map(read => read.start)).toEqual([0]);
    source.reads[0]!.complete();
    await vi.waitFor(() => expect(writes.length).toBe(1));
    expect(source.reads.map(read => read.start)).toEqual([0, 4]);
    source.reads[1]!.complete();
    await Promise.resolve(); await Promise.resolve();
    expect(source.reads.length).toBe(2);
    expect(writes.length).toBe(1);
    writes[0]!.done.resolve();
    await vi.waitFor(() => expect(writes.length).toBe(2));
    expect(source.reads.map(read => read.start)).toEqual([0, 4, 8]);
    source.reads[2]!.complete();
    writes[1]!.done.resolve();
    await vi.waitFor(() => expect(writes.length).toBe(3));
    writes[2]!.done.resolve();
    await sending;
    expect(writes.map(write => write.offset)).toEqual([0, 4, 8]);
    expect(writes.flatMap(write => [...write.bytes])).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(source.peak()).toBe(1); expect(source.active()).toBe(0);
    expect(peakWrites).toBe(1); expect(activeWrites).toBe(0);
  });

  it('drains a speculative read before rejecting the failed write', async () => {
    const source = controlledBlob(), write = deferred<void>();
    const failure = new Error('write failed');
    let settled = false;
    const sending = writeBlobChunks(source.blob, () => write.promise, 4);
    sending.then(() => { settled = true; }, () => { settled = true; });
    const rejected = expect(sending).rejects.toBe(failure);
    source.reads[0]!.complete();
    await vi.waitFor(() => expect(source.reads.length).toBe(2));
    write.reject(failure);
    await Promise.resolve(); await Promise.resolve();
    expect(settled).toBe(false);
    source.reads[1]!.reject(new Error('speculative read failed'));
    await rejected;
    expect(source.reads.length).toBe(2);
    expect(source.active()).toBe(0);
  });

  it('preserves a write failure even when its speculative read rejected earlier', async () => {
    const source = controlledBlob(), write = deferred<void>();
    const failure = new Error('write failed');
    const sending = writeBlobChunks(source.blob, () => write.promise, 4);
    const rejected = expect(sending).rejects.toBe(failure);
    source.reads[0]!.complete();
    await vi.waitFor(() => expect(source.reads.length).toBe(2));
    source.reads[1]!.reject(new Error('earlier speculative rejection'));
    await Promise.resolve(); await Promise.resolve();
    write.reject(failure);
    await rejected;
    expect(source.reads.length).toBe(2);
    expect(source.active()).toBe(0);
  });

  it('reports a speculative read failure after the preceding write succeeds', async () => {
    const source = controlledBlob(), write = deferred<void>();
    const failure = new Error('read failed'), send = vi.fn(() => write.promise);
    const sending = writeBlobChunks(source.blob, send, 4);
    const rejected = expect(sending).rejects.toBe(failure);
    source.reads[0]!.complete();
    await vi.waitFor(() => expect(source.reads.length).toBe(2));
    source.reads[1]!.reject(failure); write.resolve();
    await rejected;
    expect(send).toHaveBeenCalledTimes(1);
    expect(source.reads.length).toBe(2);
  });

  it.each([0, 1, 4, 5, 9])('delivers a %i-byte Blob with exact single/boundary/tail bytes', async length => {
    const bytes = Uint8Array.from({ length }, (_, index) => index + 1);
    const chunks: Uint8Array[] = [], offsets: number[] = [];
    const source = new Blob([bytes]);
    const read = vi.spyOn(source, 'slice');
    await writeBlobChunks(source, async (chunk, offset) => { chunks.push(chunk); offsets.push(offset); }, 4);
    expect(chunks.flatMap(chunk => [...chunk])).toEqual([...bytes]);
    expect(offsets).toEqual(Array.from({ length: Math.ceil(length / 4) }, (_, index) => index * 4));
    expect(read).toHaveBeenCalledTimes(Math.ceil(length / 4));
  });

  it('keeps its default chunks within the existing one MiB upload limit', async () => {
    const lengths: number[] = [];
    await writeBlobChunks(new Blob([new Uint8Array(1024 * 1024 + 1)]), async bytes => { lengths.push(bytes.length); });
    expect(lengths).toEqual([1024 * 1024, 1]);
  });

  it.each(['slice', 'arrayBuffer'])('reports a synchronous %s failure without writing', async operation => {
    const failure = new Error('source unavailable'), write = vi.fn();
    const source = { size: 1, slice: () => {
      if (operation === 'slice') throw failure;
      return { arrayBuffer: () => { throw failure; } };
    } } as unknown as Blob;
    await expect(writeBlobChunks(source, write)).rejects.toBe(failure);
    expect(write).not.toHaveBeenCalled();
  });

  it.each([0, -1, Number.NaN, 1.5])('rejects invalid chunk size %s before reading', async chunkBytes => {
    const source = controlledBlob();
    await expect(writeBlobChunks(source.blob, vi.fn(), chunkBytes)).rejects.toThrow('Invalid Blob chunk size');
    expect(source.reads).toEqual([]);
  });
});
