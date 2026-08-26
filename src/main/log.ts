import type { IpcMain, IpcMainEvent } from 'electron';

import { isOneOf, isRecord, isString } from '../shared/guards';
import { IPC, LIMITS } from '../shared/ipc';

const LOG_LEVELS = ['info', 'warn', 'error', 'uncaught'] as const;
const ASCII_CONTROL_CHARACTERS = /[\x00-\x1f]/g;

export interface LogIpcContext {
  isTrustedSenderContents(sender: IpcMainEvent['sender']): boolean;
}

export function registerLogIpc(ipcMain: Pick<IpcMain, 'on'>, ctx: LogIpcContext): void {
  ipcMain.on(IPC.log, (event, payload: unknown) => {
    if (!ctx.isTrustedSenderContents(event.sender)) return;
    if (!isRecord(payload)) return;

    const level = payload['level'];
    const text = payload['text'];
    if (!isOneOf(level, LOG_LEVELS) || !isString(text, LIMITS.logChars)) return;

    const singleLineText = text.replace(ASCII_CONTROL_CHARACTERS, ' ');
    process.stderr.write(`[renderer:${level}] ${singleLineText}\n`);
  });
}

export { LOG_LEVELS };
