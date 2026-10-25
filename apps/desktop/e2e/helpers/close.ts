import type { ElectronApplication } from 'playwright';

/** Leave time inside the fixture's 60 s budget to remove its profile and
 * attach diagnostics, even when native shutdown never finishes. */
export const SHUTDOWN_MS = 45_000;

export async function closeTestProcess(app: ElectronApplication, timeout = SHUTDOWN_MS): Promise<boolean> {
  const closing = (async () => {
    // A failed close-decision assertion must not leave its Cancel mock armed.
    try { await app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); }); } catch { /* already closed */ }
    try { await app.close(); } catch { /* already closed */ }
  })();
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      closing.then(() => false),
      new Promise<boolean>(resolve => {
        timer = setTimeout(() => {
          app.process().kill('SIGKILL');
          resolve(true);
        }, timeout);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
