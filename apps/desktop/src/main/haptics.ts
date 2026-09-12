import type { IpcMain, IpcMainEvent } from 'electron';
import { triggerAlignment as triggerNativeAlignment } from '@powermove/macos-haptics';

import { IPC } from '../shared/ipc';

const MIN_HAPTIC_INTERVAL_MS = 80;

export interface HapticsIpcContext {
  isTrustedSenderContents(sender: IpcMainEvent['sender']): boolean;
  triggerAlignment?: () => void;
  now?: () => number;
}

export function registerHapticsIpc(
  ipcMain: Pick<IpcMain, 'on'>,
  ctx: HapticsIpcContext
): void {
  const triggerAlignment = ctx.triggerAlignment ?? triggerNativeAlignment;
  const now = ctx.now ?? Date.now;
  let lastFeedbackAt = -Infinity;

  ipcMain.on(IPC.hapticAlignment, (event) => {
    if (!ctx.isTrustedSenderContents(event.sender)) return;

    const requestedAt = now();
    if (requestedAt - lastFeedbackAt < MIN_HAPTIC_INTERVAL_MS) return;
    lastFeedbackAt = requestedAt;

    try {
      triggerAlignment();
    } catch (error) {
      // Haptics are optional hardware feedback; never interrupt a drag if the
      // device or native performer is unavailable.
      process.stderr.write(`[haptics] ${error instanceof Error ? error.message : String(error)}\n`);
    }
  });
}

export { MIN_HAPTIC_INTERVAL_MS };
