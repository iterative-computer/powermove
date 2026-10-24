/*
 * Getting a window's engine connection back (contract 7; docs/engine/host-processes.md,
 * "Crash handling").
 *
 * Main gives a window a new port with every engine launch, so a restart
 * reconnects by itself. A connection that closes while its launch lives
 * gets nothing from main: the engine closed it (a message it refused fails
 * the whole connection, RFC 09), or the port was lost. The window then asks
 * main for fresh ports (`requestPorts`) with the supervisor's backoff:
 * 250 ms doubling to 30 s, reset once a connection lasted a minute. Asks
 * repeat until a port connects; one that arrives cancels the rest.
 */

/** The first ask, the longest wait between asks, how long a connection must
 *  last to reset the backoff, and how long to wait for main's port. */
export const RECONNECT_FIRST_MS = 250;
export const RECONNECT_MAX_MS = 30_000;
export const RECONNECT_RESET_MS = 60_000;
export const RECONNECT_WAIT_MS = 2_000;

export type ReconnectClock = {
  now(): number;
  later(fn: () => void, ms: number): unknown;
  cancel(timer: unknown): void;
};

const realClock: ReconnectClock = {
  now: () => performance.now(),
  later: (fn, ms) => setTimeout(fn, ms),
  cancel: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>)
};

export class PortReconnector {
  private timer: unknown = null;
  private delay = RECONNECT_FIRST_MS;
  private openedAt = 0;
  private disposed = false;
  /** Asks made since the last connection (diagnostics). */
  asks = 0;

  constructor(
    /** Asks main for fresh ports. */
    private readonly ask: () => void,
    /** Whether the window has a live connection now. */
    private readonly connected: () => boolean,
    private readonly clock: ReconnectClock = realClock
  ) {}

  /** A port arrived: whatever was scheduled is moot. */
  portArrived(): void {
    this.clear();
  }

  /** A connection finished its handshake. */
  opened(): void {
    this.clear();
    this.openedAt = this.clock.now();
    this.asks = 0;
  }

  /** The window's current connection closed. */
  closed(): void {
    if (this.disposed || this.timer !== null) return;
    if (this.openedAt && this.clock.now() - this.openedAt >= RECONNECT_RESET_MS) this.delay = RECONNECT_FIRST_MS;
    this.openedAt = 0;
    this.schedule();
  }

  dispose(): void {
    this.disposed = true;
    this.clear();
  }

  private schedule(): void {
    const delay = this.delay;
    this.delay = Math.min(RECONNECT_MAX_MS, this.delay * 2);
    this.timer = this.clock.later(() => {
      this.timer = null;
      if (this.disposed || this.connected()) return;
      this.asks++;
      this.ask();
      // No port came (the engine is restarting, or main has none yet).
      this.timer = this.clock.later(() => {
        this.timer = null;
        if (!this.disposed && !this.connected()) this.schedule();
      }, RECONNECT_WAIT_MS);
    }, delay);
  }

  private clear(): void {
    if (this.timer !== null) this.clock.cancel(this.timer);
    this.timer = null;
  }
}
