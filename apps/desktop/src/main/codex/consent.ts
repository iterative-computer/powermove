import { randomBytes } from 'node:crypto';

import type { BrowserWindow, MessageBoxOptions, MessageBoxReturnValue } from 'electron';
import type { ConsentRequest, ConsentResult } from '../../shared/ipc';

export const COMPUTER_CONSENT_TTL_MS = 5 * 60 * 1000;

interface TokenRecord {
  expiresAt: number;
}

const tokens = new Map<string, TokenRecord>();

export interface ConsentDialog {
  showMessageBox(window: BrowserWindow, options: MessageBoxOptions): Promise<MessageBoxReturnValue>;
}

export interface ConsentDependencies {
  dialog?: ConsentDialog;
  now?: () => number;
  randomBytes?: (size: number) => Uint8Array;
}

/**
 * Ask in a window-modal native dialog, then mint an in-memory, one-run token.
 * The token confers no authority until consumeToken removes it successfully.
 */
export async function requestComputerConsent(
  window: BrowserWindow,
  request: ConsentRequest,
  dependencies: ConsentDependencies = {}
): Promise<ConsentResult> {
  const consentDialog = dependencies.dialog ?? (await import('electron')).dialog;
  const response = await consentDialog.showMessageBox(window, {
    type: 'warning',
    buttons: ['Cancel', 'Allow this run'],
    defaultId: 0,
    cancelId: 0,
    noLink: true,
    title: 'Allow computer access?',
    message: `Allow computer access for “${request.projectName}”?`,
    detail: request.summary
  });
  if (response.response !== 1) return { granted: false };

  const now = (dependencies.now ?? Date.now)();
  const bytes = (dependencies.randomBytes ?? randomBytes)(32);
  const token = Buffer.from(bytes).toString('hex');
  const expiresAt = now + COMPUTER_CONSENT_TTL_MS;
  tokens.set(token, { expiresAt });
  return { granted: true, token, expiresAt };
}

/** Consume exactly once. Expired tokens are removed as they are observed. */
export function consumeToken(token: string, now = Date.now()): boolean {
  const record = tokens.get(token);
  if (record === undefined) return false;
  tokens.delete(token);
  return now < record.expiresAt;
}

/** Test and shutdown hygiene; consent is deliberately never persisted. */
export function clearConsentTokens(): void {
  tokens.clear();
}
