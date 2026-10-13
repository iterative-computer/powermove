import type { BrowserWindow, Dialog } from 'electron';

/** A native prompt remains usable after the editor process has disappeared. */
export function installRendererRecovery(window: BrowserWindow, dialog: Pick<Dialog, 'showMessageBox'>): void {
  let prompting = false;
  window.webContents.on('render-process-gone', (_event, details) => {
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
