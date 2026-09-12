import { describe, expect, it, vi } from 'vitest';

import { IPC } from '../shared/ipc';
import { registerNativeEditIpc } from './native-edit';

describe('native text edit IPC', () => {
  it('dispatches only allowlisted actions from the trusted editor', () => {
    let listener: ((event: any, value: unknown) => void) | undefined;
    const ipcMain = { on: vi.fn((_channel, value) => { listener = value; }) };
    const sender = {
      paste: vi.fn(), cut: vi.fn(), copy: vi.fn(), undo: vi.fn(), redo: vi.fn(), selectAll: vi.fn(),
    };
    registerNativeEditIpc(ipcMain as never, { isTrustedSenderContents: () => true });
    expect(ipcMain.on).toHaveBeenCalledWith(IPC.nativeEdit, expect.any(Function));

    listener?.({ sender }, 'paste');
    listener?.({ sender }, 'delete');
    expect(sender.paste).toHaveBeenCalledOnce();
    expect(sender.cut).not.toHaveBeenCalled();
  });

  it('rejects actions from an untrusted WebContents', () => {
    let listener: ((event: any, value: unknown) => void) | undefined;
    const ipcMain = { on: vi.fn((_channel, value) => { listener = value; }) };
    const sender = { paste: vi.fn() };
    registerNativeEditIpc(ipcMain as never, { isTrustedSenderContents: () => false });
    listener?.({ sender }, 'paste');
    expect(sender.paste).not.toHaveBeenCalled();
  });
});
