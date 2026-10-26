/** Cancelling Close abandons the close request, never an already-started file write. */
export class ClosePreparation {
  readonly controller = new AbortController();
  constructor(readonly id: string, private readonly report: (detail: string) => void) {}

  cancel(reason = 'Close cancelled'): void {
    if (!this.controller.signal.aborted) this.controller.abort(new Error(reason));
  }

  check(): void { this.controller.signal.throwIfAborted(); }

  async wait<T>(detail: string, operation: () => T | PromiseLike<T>, timeout = 60_000): Promise<T> {
    this.check();
    this.report(detail);
    const signal = this.controller.signal;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let onAbort!: () => void;
    const interrupted = new Promise<never>((_resolve, reject) => {
      onAbort = () => reject(signal.reason);
      signal.addEventListener('abort', onAbort, { once: true });
      // Human decisions/file dialogs have no deadline. A stalled work phase
      // cancels preparation and leaves the editor open; it never authorizes exit.
      if (timeout) timer = setTimeout(() => this.cancel('Close preparation timed out. Your project is still open.'), timeout);
    });
    try {
      const value = await Promise.race([Promise.resolve().then(() => { this.check(); return operation(); }), interrupted]);
      this.check();
      return value;
    } finally {
      clearTimeout(timer);
      signal.removeEventListener('abort', onAbort);
    }
  }
}
