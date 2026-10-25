/** A resync rejected before admission may retry with the same staged body.
 * Other errors remain failures; close/restart ends the retry at the caller. */
export async function retryResync<T>(send: () => Promise<T>, signal?: AbortSignal): Promise<T> {
  for (;;) {
    if (signal?.aborted) throw new Error('The resync connection closed.');
    try { return await send(); }
    catch (error) {
      const retry = (error as { error?: { retry?: { kind?: string; ms?: number } } } | null)?.error?.retry;
      if (retry?.kind !== 'after' || typeof retry.ms !== 'number' || !Number.isFinite(retry.ms) || retry.ms < 0) throw error;
      await new Promise<void>((resolve, reject) => {
        const cancel = () => { clearTimeout(timer); signal?.removeEventListener('abort', cancel); reject(new Error('The resync connection closed.')); };
        const timer = setTimeout(() => { signal?.removeEventListener('abort', cancel); resolve(); }, Math.min(0x7fffffff, retry.ms!));
        signal?.addEventListener('abort', cancel, { once: true });
        if (signal?.aborted) cancel();
      });
    }
  }
}
