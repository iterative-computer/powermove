import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({
  BrowserWindow: { fromWebContents: vi.fn(() => ({ id: 1, isDestroyed: () => false })) },
  dialog: { showMessageBox: vi.fn() }
}));

import { confirmOptions, registerConfirmIpc } from './native-confirm';
import { IPC } from '../shared/ipc';

function register(response: number, trusted = true) {
  const handlers = new Map<string, (event: any, request: any) => Promise<unknown>>();
  const showMessageBox = vi.fn(async () => ({ response, checkboxChecked: false }));
  registerConfirmIpc({ handle: (channel: string, handler: any) => { handlers.set(channel, handler); } } as any, {
    isTrustedSender: () => trusted,
    showMessageBox: showMessageBox as any
  });
  return { handle: handlers.get(IPC.dialogConfirm)!, showMessageBox };
}

describe('confirmOptions', () => {
  it('puts the action first, Cancel second, and defaults to the action', () => {
    expect(confirmOptions({ message: 'Remove?', detail: 'Gone.', confirmLabel: 'Remove' })).toEqual({
      type: 'question', message: 'Remove?', detail: 'Gone.',
      buttons: ['Remove', 'Cancel'], defaultId: 0, cancelId: 1, noLink: true
    });
  });

  it('defaults destructive prompts to Cancel and falls back to OK', () => {
    expect(confirmOptions({ message: 'Delete forever?', destructive: true })).toMatchObject({
      type: 'warning', buttons: ['OK', 'Cancel'], defaultId: 1, cancelId: 1
    });
  });
});

describe('registerConfirmIpc', () => {
  it('resolves true only for the primary button', async () => {
    const yes = register(0);
    await expect(yes.handle({ sender: {} }, { message: 'Go?' })).resolves.toBe(true);
    expect(yes.showMessageBox).toHaveBeenCalledWith({ id: 1, isDestroyed: expect.any(Function) }, expect.objectContaining({ message: 'Go?' }));
    const no = register(1);
    await expect(no.handle({ sender: {} }, { message: 'Go?' })).resolves.toBe(false);
  });

  it('rejects untrusted senders and empty messages', async () => {
    await expect(register(0, false).handle({ sender: {} }, { message: 'Go?' })).rejects.toThrow('Unauthorized');
    await expect(register(0).handle({ sender: {} }, { message: '   ' })).rejects.toThrow('message');
  });
});
