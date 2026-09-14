import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({
  BrowserWindow: { fromWebContents: vi.fn(() => ({ id: 1 })) },
  Menu: { buildFromTemplate: vi.fn() },
  nativeImage: { createFromDataURL: vi.fn(), createEmpty: vi.fn() }
}));

import { nativeMenuTemplate, registerContextMenuIpc, sanitizeNativeMenu } from './context-menu';
import { IPC } from '../shared/ipc';

const PNG = 'data:image/png;base64,iVBORw0KGgo=';

function register(popup: (template: any[], window: unknown, at: unknown) => Promise<void>, trusted = true) {
  const handlers = new Map<string, (event: any, request: any) => Promise<unknown>>();
  const image = vi.fn((dataURL: string) => ({ template: dataURL }) as any);
  registerContextMenuIpc({ handle: (channel: string, handler: any) => { handlers.set(channel, handler); } } as any, {
    isTrustedSenderContents: () => trusted,
    popup,
    image
  });
  return { handle: handlers.get(IPC.menuPopup)!, image };
}

describe('registerContextMenuIpc', () => {
  it('shows the menu at the pointer with template icons and resolves with the clicked id', async () => {
    const popup = vi.fn(async (template: any[]) => { template[1].click(); });
    const { handle, image } = register(popup);
    const result = await handle({ sender: {} }, {
      x: 40.4, y: 80,
      items: [
        { type: 'header', id: 'h', label: 'Layer' },
        { type: 'normal', id: 'rename', label: 'Rename', accelerator: 'F2', icon: PNG },
        { type: 'separator' }
      ]
    });
    expect(result).toBe('rename');
    expect(popup).toHaveBeenCalledWith(expect.any(Array), { id: 1 }, { x: 40.4, y: 80 });
    const template = popup.mock.calls[0]![0];
    expect(template[0]).toEqual({ label: 'Layer', enabled: false });
    expect(template[1]).toMatchObject({ type: 'normal', label: 'Rename', accelerator: 'F2', enabled: true, registerAccelerator: false, icon: { template: PNG } });
    expect(template[2]).toEqual({ type: 'separator' });
    expect(image).toHaveBeenCalledWith(PNG);
  });

  it('resolves null when the menu closes without a choice', async () => {
    const { handle } = register(async () => undefined);
    expect(await handle({ sender: {} }, { items: [{ type: 'normal', id: 'a', label: 'A' }] })).toBeNull();
  });

  it('skips menus with nothing selectable and rejects untrusted senders', async () => {
    const popup = vi.fn(async () => undefined);
    const { handle } = register(popup);
    expect(await handle({ sender: {} }, { items: [{ type: 'header', id: 'h', label: 'Only' }, { type: 'separator' }] })).toBeNull();
    expect(popup).not.toHaveBeenCalled();
    await expect(register(popup, false).handle({ sender: {} }, { items: [{ type: 'normal', id: 'a', label: 'A' }] })).rejects.toThrow('Unauthorized');
  });
});

describe('sanitizeNativeMenu', () => {
  it('keeps only well-formed rows and PNG data-URL icons', () => {
    expect(sanitizeNativeMenu([
      null, 'x', { type: 'bogus', id: 'a', label: 'A' }, { type: 'normal', id: 1, label: 'A' },
      { type: 'checkbox', id: 'c', label: 'C', checked: true, accelerator: '', icon: PNG },
      { type: 'normal', id: 'n', label: 'N', icon: 'https://evil.example/x.png' },
      { type: 'normal', id: 'j', label: 'J', icon: 'data:image/jpeg;base64,AAAA' },
      { type: 'separator' }
    ])).toEqual([
      { type: 'checkbox', id: 'c', label: 'C', enabled: true, checked: true, accelerator: undefined, icon: PNG },
      { type: 'normal', id: 'n', label: 'N', enabled: true, checked: false, accelerator: undefined, icon: undefined },
      { type: 'normal', id: 'j', label: 'J', enabled: true, checked: false, accelerator: undefined, icon: undefined },
      { type: 'separator' }
    ]);
  });
});

describe('nativeMenuTemplate', () => {
  it('reports the chosen id through the click handler and leaves iconless rows bare', () => {
    const clicked: string[] = [];
    const template = nativeMenuTemplate([{ type: 'checkbox', id: 'shy', label: 'Shy', checked: true }], (id) => clicked.push(id), () => undefined);
    expect(template[0]).toMatchObject({ type: 'checkbox', checked: true, icon: undefined });
    (template[0] as any).click();
    expect(clicked).toEqual(['shy']);
  });
});
