import { afterEach, describe, expect, it, vi } from 'vitest';

import type { BrowserWindow } from 'electron';
import {
  clearConsentTokens,
  COMPUTER_CONSENT_TTL_MS,
  consumeToken,
  requestComputerConsent,
  revokeStandingConsent,
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

  it('asks once for standing Full access, then mints tokens without asking until revoked', async () => {
    const showMessageBox = vi.fn(async () => ({ response: 1, checkboxChecked: false }));
    const dependencies = { dialog: { showMessageBox } as ConsentDialog };
    const first = await requestComputerConsent(windowStub, { ...request, standing: true }, dependencies);
    expect(first.granted).toBe(true);
    expect(showMessageBox).toHaveBeenCalledWith(windowStub, expect.objectContaining({ buttons: ['Cancel', 'Allow Full Access'] }));

    const second = await requestComputerConsent(windowStub, { ...request, standing: true }, dependencies);
    expect(second.granted).toBe(true);
    expect(showMessageBox).toHaveBeenCalledOnce();
    if (second.granted) expect(consumeToken(second.token)).toBe(true);

    // A one-run request still asks, and revoking makes the next standing request ask again.
    await requestComputerConsent(windowStub, request, dependencies);
    expect(showMessageBox).toHaveBeenCalledTimes(2);
    revokeStandingConsent();
    await requestComputerConsent(windowStub, { ...request, standing: true }, dependencies);
    expect(showMessageBox).toHaveBeenCalledTimes(3);
  });

  it('holds no standing grant when the Full access dialog is cancelled', async () => {
    const showMessageBox = vi.fn(async () => ({ response: 0, checkboxChecked: false }));
    const dependencies = { dialog: { showMessageBox } as ConsentDialog };
    expect(await requestComputerConsent(windowStub, { ...request, standing: true }, dependencies)).toEqual({ granted: false });
    expect(await requestComputerConsent(windowStub, { ...request, standing: true }, dependencies)).toEqual({ granted: false });
    expect(showMessageBox).toHaveBeenCalledTimes(2);
  });
});
