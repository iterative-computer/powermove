/** Prepare only the next bounded Blob slice while its predecessor is written.
 * Writes stay sequential; no speculative read outlives completion or failure. */
export async function writeBlobChunks(blob: Blob,
  write: (bytes: Uint8Array<ArrayBuffer>, offset: number) => Promise<void>, chunkBytes = 1024 * 1024, signal?: AbortSignal): Promise<void> {
  if (!Number.isSafeInteger(chunkBytes) || chunkBytes < 1) throw new RangeError('Invalid Blob chunk size');
  signal?.throwIfAborted();
  if (!blob.size) return;
  type Read = { ok: true; bytes: Uint8Array<ArrayBuffer> } | { ok: false; error: unknown };
  const read = async (offset: number): Promise<Read> => {
    try { signal?.throwIfAborted(); return { ok: true, bytes: new Uint8Array(await blob.slice(offset, offset + chunkBytes).arrayBuffer()) }; }
    catch (error) { return { ok: false, error }; }
  };
  let pending: Promise<Read> | undefined = read(0);
  try {
    for (let offset = 0; offset < blob.size; offset += chunkBytes) {
      const result = await pending!;
      pending = undefined;
      signal?.throwIfAborted();
      if (!result.ok) throw result.error;
      if (result.bytes.length !== Math.min(chunkBytes, blob.size - offset)) throw new Error('The project media is incomplete.');
      if (offset + chunkBytes < blob.size) pending = read(offset + chunkBytes);
      await write(result.bytes, offset);
    }
  } finally {
    // The settled result also prevents an early speculative rejection from
    // becoming unhandled or replacing the acknowledged write's actual error.
    await pending;
  }
}
