import type { BrowserWindow, Dialog, WebContents } from 'electron';

const unavailableRenderers = new WeakSet<WebContents>();
const pendingReloads = new WeakSet<BrowserWindow>();

/** Operations belong to one document, even when navigation reuses WebContents. */
export function watchRendererDocument(contents: WebContents, onLoss: () => void): () => void {
  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    contents.removeListener('destroyed', lose);
    contents.removeListener('render-process-gone', lose);
    contents.removeListener('did-start-navigation', navigate);
  };
  const lose = () => { if (!stopped) { stop(); onLoss(); } };
  const navigate = (details: { isMainFrame: boolean; isSameDocument: boolean }) => {
    if (details.isMainFrame && !details.isSameDocument) lose();
  };
  contents.on('destroyed', lose);
  contents.on('render-process-gone', lose);
  contents.on('did-start-navigation', navigate);
  return stop;
}

export function isRendererUnavailable(contents: WebContents): boolean {
  return contents.isDestroyed() || contents.isCrashed?.() === true || unavailableRenderers.has(contents);
}

/** Recover the existing editor so its project tabs and local recovery survive. */
export function recoverRenderer(window: BrowserWindow): boolean {
  if (window.isDestroyed() || window.webContents.isDestroyed() || !isRendererUnavailable(window.webContents)) return false;
  if (!pendingReloads.has(window)) {
    pendingReloads.add(window);
    setImmediate(() => {
      pendingReloads.delete(window);
      if (!window.isDestroyed() && !window.webContents.isDestroyed() && isRendererUnavailable(window.webContents)) window.webContents.reload();
    });
  }
  return true;
}

/** A native prompt remains usable after the editor process has disappeared. */
export function installRendererRecovery(window: BrowserWindow, dialog: Pick<Dialog, 'showMessageBox'>, onUnavailable: () => void = () => {}): void {
  let prompting = false;
  window.webContents.on('did-finish-load', () => unavailableRenderers.delete(window.webContents));
  window.webContents.on('render-process-gone', (_event, details) => {
    unavailableRenderers.add(window.webContents);
    onUnavailable();
    if (details.reason === 'clean-exit' || prompting || window.isDestroyed()) return;
    prompting = true;
    console.error(`[editor] renderer exited: ${details.reason} (${details.exitCode})`);
    // Defer until Chromium finishes tearing down the old process. Navigation
    // directly inside this event has crashed some Electron versions.
    setImmediate(() => {
      if (window.isDestroyed()) { prompting = false; return; }
      void dialog.showMessageBox(window, {
        type: 'warning',
        message: 'Powermove needs to reload this window',
        detail: `${details.reason === 'oom' ? 'The editor ran out of memory.' : 'The editor stopped unexpectedly.'} Reload restores the latest local recovery. The most recent edits may be missing.`,
        buttons: ['Reload', 'Close Window'],
        defaultId: 0,
        cancelId: 1,
        noLink: true,
      }).then(({ response }) => {
        if (window.isDestroyed() || window.webContents.isDestroyed()) return;
        // Another action may have already recovered the window while prompting.
        if (!window.webContents.isCrashed()) return;
        if (response === 0) window.webContents.reload();
        else window.close();
      }).catch(error => {
        console.error('[editor] recovery prompt failed', error);
      }).finally(() => { prompting = false; });
    });
  });
}
