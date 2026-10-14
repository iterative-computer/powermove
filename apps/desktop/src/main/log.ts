import { appendFileSync } from 'node:fs';

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
    nativeReport(singleLineText);
  });
}

/**
 * The native evaluator's test reports (`[pm-native]` lines from
 * engine-doc/report.ts) also go to `POWERMOVE_NATIVE_REPORT_FILE` when a test
 * run sets it, so a suite of launches can be summed afterwards
 * (scripts/native-evaluator-report.mjs).
 */
function nativeReport(line: string): void {
  const file = process.env.POWERMOVE_NATIVE_REPORT_FILE;
  if (!file || !line.startsWith('[pm-native] ')) return;
  try { appendFileSync(file, `${line.slice(12)}\n`); } catch { /* reporting never breaks the app */ }
}

export { LOG_LEVELS };
