/*
 * Returning a window's freed memory after a large document preparation.
 *
 * Preparing a large document (clone, JSON text, ingestion) leaves the
 * renderer's V8 heap and PartitionAlloc with pages that are free but still
 * resident: for the large private project its RSS stayed about 450 MB above
 * today's although its live heap after GC was the same (host-processes.md).
 * Chromium returns such pages when the system reports memory pressure; this
 * reports a moderate one to that window's renderer (the DevTools protocol's
 * Memory.simulatePressureNotification, through Electron's debugger API, which
 * neither shows nor changes anything else), once the preparation has settled.
 */
export interface ReliefTarget {
  isDestroyed(): boolean;
  readonly debugger: {
    isAttached(): boolean;
    attach(protocolVersion?: string): void;
    detach(): void;
    sendCommand(method: string, params?: object): Promise<unknown>;
  };
}

/** Documents at least this large (bytes of JSON) are followed by a relief. */
export const RELIEF_MIN_BYTES = 64 * 1024 * 1024;

export function createMemoryRelief(options: { delayMs?: number; warn?: (message: string) => void } = {}) {
  const delay = options.delayMs ?? 5_000;
  const timers = new Map<ReliefTarget, ReturnType<typeof setTimeout>>();
  const relieve = async (target: ReliefTarget): Promise<void> => {
    if (target.isDestroyed()) return;
    const attached = target.debugger.isAttached();
    try {
      if (!attached) target.debugger.attach('1.3');
      await target.debugger.sendCommand('Memory.simulatePressureNotification', { level: 'moderate' });
    } catch (error) {
      // DevTools may hold the session, or the window may be closing: the
      // relief is an optimization and never an error for the open.
      options.warn?.(`memory relief skipped: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      if (!attached && !target.isDestroyed()) { try { target.debugger.detach(); } catch { /* already detached */ } }
    }
  };
  return {
    /** Relieves `target` once no later document has arrived for `delayMs`. */
    schedule(target: ReliefTarget): void {
      clearTimeout(timers.get(target));
      const timer = setTimeout(() => { timers.delete(target); void relieve(target); }, delay);
      timer.unref?.();
      timers.set(target, timer);
    },
    dispose(): void {
      for (const timer of timers.values()) clearTimeout(timer);
      timers.clear();
    },
  };
}
