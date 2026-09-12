import { afterEach, describe, expect, it, vi } from 'vitest';

import type { BrowserWindow } from 'electron';
import {
  clearConsentTokens,
  COMPUTER_CONSENT_TTL_MS,
  consumeToken,
  requestComputerConsent,
  type ConsentDialog
} from './consent';

const windowStub = {} as BrowserWindow;
const request = { projectName: 'Demo', summary: 'Open and operate another application.' };

afterEach(() => clearConsentTokens());

describe('computer consent', () => {
  it('uses a window-modal native dialog and returns false when cancelled', async () => {
    const showMessageBox = vi.fn(async () => ({ response: 0, checkboxChecked: false }));
    const result = await requestComputerConsent(windowStub, request, {
      dialog: { showMessageBox } as ConsentDialog
    });

    expect(result).toEqual({ granted: false });
    expect(showMessageBox).toHaveBeenCalledWith(
      windowStub,
      expect.objectContaining({
        buttons: ['Cancel', 'Allow this run'],
        defaultId: 0,
        cancelId: 0,
        detail: request.summary
      })
    );
  });

  it('mints a 32-byte hex token valid for five minutes and consumes it once', async () => {
    const now = 10_000;
    const result = await requestComputerConsent(windowStub, request, {
      dialog: { showMessageBox: async () => ({ response: 1, checkboxChecked: false }) },
      now: () => now,
      randomBytes: (size) => new Uint8Array(size).fill(0xab)
    });

    expect(result).toEqual({
      granted: true,
      token: 'ab'.repeat(32),
      expiresAt: now + COMPUTER_CONSENT_TTL_MS
    });
    if (!result.granted) throw new Error('Expected granted consent.');
    expect(consumeToken(result.token, result.expiresAt - 1)).toBe(true);
    expect(consumeToken(result.token, result.expiresAt - 1)).toBe(false);
  });

  it('rejects and removes an expired token', async () => {
    const result = await requestComputerConsent(windowStub, request, {
      dialog: { showMessageBox: async () => ({ response: 1, checkboxChecked: false }) },
      now: () => 1,
      randomBytes: (size) => new Uint8Array(size).fill(0xcd)
    });
    if (!result.granted) throw new Error('Expected granted consent.');

    expect(consumeToken(result.token, result.expiresAt)).toBe(false);
    expect(consumeToken(result.token, result.expiresAt - 1)).toBe(false);
  });
});
