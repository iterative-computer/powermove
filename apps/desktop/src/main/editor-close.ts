import type { BrowserWindow } from 'electron';

/** Share native Close and Quit preparation, including across overlapping requests. */
export function createEditorClosePreparation(allowed: (window: BrowserWindow) => boolean) {
  const pending = new Map<BrowserWindow, Promise<void>>();
  const prepared = new Set<BrowserWindow>();
  let nextId = 0;
  const usable = (window: BrowserWindow) => !window.isDestroyed() && !window.webContents.isDestroyed()
    && !window.webContents.isCrashed() && allowed(window);
  const evaluate = (window: BrowserWindow, code: string) => Promise.resolve().then(() => window.webContents.executeJavaScript(code));
  const cancel = (window: BrowserWindow) => {
    prepared.delete(window);
    if (usable(window)) void evaluate(window, 'window.PM?.cancelClosePreparation?.()').catch(() => undefined);
  };
  return {
    prepare(window: BrowserWindow): Promise<void> {
      if (!usable(window)) return Promise.resolve();
      const existing = pending.get(window);
      if (existing) return existing;
      const id = `native-close-${++nextId}`;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const work = Promise.race([
        evaluate(window, `window.PM?.prepareToClose?.(${JSON.stringify({ id, flush: true })}) ?? true`),
        new Promise<never>((_resolve, reject) => {
          timer = setTimeout(() => {
            // Scope cancellation to this attempt: a delayed callback must never
            // cancel a newer request or close a window after the timeout.
            if (usable(window)) void evaluate(window, `window.PM?.cancelClosePreparation?.(${JSON.stringify(id)})`).catch(() => undefined);
            reject(new Error('Close preparation timed out. Your project is still open.'));
          }, 120_000);
        }),
      ]).then(result => {
        if (!result) throw new Error('Close cancelled');
        prepared.add(window);
      }).finally(() => { clearTimeout(timer); pending.delete(window); });
      pending.set(window, work);
      return work;
    },
    async commit(windows: BrowserWindow[]): Promise<void> {
      // Validate every editor before allowing any of them to unload.
      for (const window of windows) {
        if (!usable(window)) continue;
        if (!prepared.has(window) || !await window.webContents.executeJavaScript('window.PM?.commitClosePreparation?.() ?? true')) {
          throw new Error('Close cancelled');
        }
      }
    },
    cancel,
    forget(window: BrowserWindow): void { prepared.delete(window); },
    cancelAll(): void { for (const window of new Set([...prepared, ...pending.keys()])) cancel(window); },
  };
}
