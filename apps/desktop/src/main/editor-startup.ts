import type { BrowserWindow, WebContents } from 'electron';

/** A failed first launch must release setup instead of leaving a blank window. */
export function openOnboardingEditor(create: (ready: () => void) => BrowserWindow, timeoutMs = 30_000): Promise<BrowserWindow> {
  return new Promise((resolve, reject) => {
    let window: BrowserWindow | undefined;
    let contents: WebContents | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let settled = false, readyBeforeCreate = false;
    const cleanup = () => {
      clearTimeout(timer);
      window?.removeListener('closed', closed);
      contents?.removeListener('did-fail-load', failed);
      contents?.removeListener('render-process-gone', crashed);
    };
    const fail = (error: Error, destroy = true) => {
      if (settled) return;
      settled = true; cleanup();
      if (destroy && window && !window.isDestroyed()) window.destroy();
      reject(error);
    };
    const ready = () => {
      if (!window) { readyBeforeCreate = true; return; }
      if (settled) return;
      settled = true; cleanup(); resolve(window);
    };
    const closed = () => fail(new Error('The editor was closed before it finished opening.'), false);
    const failed = (_event: unknown, _code: number, description: string, _url: string, mainFrame: boolean) => {
      if (mainFrame !== false) fail(new Error(description));
    };
    const crashed = () => fail(new Error('The editor stopped before it finished opening.'));
    window = create(ready);
    contents = window.webContents;
    window.once('closed', closed);
    contents.on('did-fail-load', failed);
    contents.once('render-process-gone', crashed);
    timer = setTimeout(() => fail(new Error('Powermove could not finish opening. Please try again.')), timeoutMs);
    if (readyBeforeCreate) ready();
  });
}
