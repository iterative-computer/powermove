import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MenuItemConstructorOptions } from 'electron';

const electronMocks = vi.hoisted(() => ({
  app: { isPackaged: false, name: 'Powermove' },
  buildFromTemplate: vi.fn((template: MenuItemConstructorOptions[]) => ({ template })),
  setApplicationMenu: vi.fn(),
  getFocusedWindow: vi.fn()
}));

vi.mock('electron', () => ({
  app: electronMocks.app,
  BrowserWindow: { getFocusedWindow: electronMocks.getFocusedWindow },
  Menu: {
    buildFromTemplate: electronMocks.buildFromTemplate,
    setApplicationMenu: electronMocks.setApplicationMenu
  }
}));

import { appMenuTemplate, buildAppMenu, installMenu } from './menu';
import { IPC, type MenuCommand } from '../shared/ipc';

function submenu(item: MenuItemConstructorOptions): MenuItemConstructorOptions[] {
  if (!Array.isArray(item.submenu)) throw new Error(`Expected submenu for ${item.label ?? 'item'}`);
  return item.submenu;
}

function topLevel(
  template: MenuItemConstructorOptions[],
  label: string
): MenuItemConstructorOptions {
  const item = template.find((candidate) => candidate.label === label);
  if (!item) throw new Error(`Missing ${label} menu`);
  return item;
}

beforeEach(() => {
  vi.clearAllMocks();
  electronMocks.app.isPackaged = false;
});

describe('application menu', () => {
  it('has the requested labels, accelerators, roles, and custom history commands', () => {
    const sent: MenuCommand[] = [];
    const template = appMenuTemplate((command) => sent.push(command));
    expect(template.map((item) => item.label)).toEqual([
      'Powermove',
      'File',
      'Edit',
      'View',
      'Window'
    ]);

    const appItems = submenu(topLevel(template, 'Powermove'));
    expect(appItems.filter((item) => item.role).map((item) => item.role)).toEqual([
      'about',
      'services',
      'hide',
      'hideOthers',
      'quit'
    ]);
    expect(appItems.find((item) => item.role === 'hide')?.accelerator).toBe('Command+H');
    expect(appItems.find((item) => item.role === 'hideOthers')?.accelerator).toBe(
      'Command+Alt+H'
    );
    expect(appItems.find((item) => item.role === 'quit')?.accelerator).toBe('Command+Q');

    const fileItems = submenu(topLevel(template, 'File'));
    expect(fileItems.map((item) => item.label ?? item.type)).toEqual([
      'New Project',
      'Open Project…',
      'Save Project',
      'Save Project As…',
      'separator',
      'Export…'
    ]);
    expect(fileItems.filter((item) => item.click).map((item) => item.accelerator)).toEqual([
      'CommandOrControl+N',
      'CommandOrControl+O',
      'CommandOrControl+S',
      'CommandOrControl+Shift+S',
      'CommandOrControl+E'
    ]);
    for (const item of fileItems) {
      item.click?.({} as never, undefined, {} as never);
    }
    expect(sent).toEqual(['newProject', 'open', 'save', 'saveAs', 'export']);

    const editItems = submenu(topLevel(template, 'Edit'));
    const undo = editItems.find((item) => item.label === 'Undo');
    const redo = editItems.find((item) => item.label === 'Redo');
    const copy = editItems.find((item) => item.label === 'Copy');
    const paste = editItems.find((item) => item.label === 'Paste');
    expect(undo).toMatchObject({ accelerator: 'CommandOrControl+Z' });
    expect(redo).toMatchObject({ accelerator: 'CommandOrControl+Shift+Z' });
    expect(undo).not.toHaveProperty('role');
    expect(redo).not.toHaveProperty('role');
    expect(copy).toMatchObject({ accelerator: 'CommandOrControl+C', id: 'copy' });
    expect(paste).toMatchObject({ accelerator: 'CommandOrControl+V', id: 'paste' });
    undo?.click?.({} as never, undefined, {} as never);
    redo?.click?.({} as never, undefined, {} as never);
    copy?.click?.({} as never, undefined, {} as never);
    paste?.click?.({} as never, undefined, {} as never);
    expect(sent).toEqual(['newProject', 'open', 'save', 'saveAs', 'export', 'undo', 'redo', 'copy', 'paste']);
    expect(editItems.filter((item) => item.role).map((item) => item.role)).toEqual([
      'cut',
      'selectAll'
    ]);

    expect(topLevel(template, 'Window').role).toBe('window');
    expect(submenu(topLevel(template, 'Window')).filter((item) => item.role).map((item) => item.role))
      .toEqual(['minimize', 'zoom', 'front']);
  });

  it('includes development view tools only in unpackaged builds', () => {
    const development = submenu(topLevel(appMenuTemplate(() => undefined), 'View'));
    expect(development.filter((item) => item.role).map((item) => item.role)).toEqual([
      'togglefullscreen',
      'reload',
      'toggleDevTools'
    ]);

    electronMocks.app.isPackaged = true;
    const packaged = submenu(topLevel(appMenuTemplate(() => undefined), 'View'));
    expect(packaged).toEqual([{ role: 'togglefullscreen' }]);
  });

  it('builds and installs the menu, keeping app commands on the main editor window', () => {
    buildAppMenu(() => undefined);
    expect(electronMocks.buildFromTemplate).toHaveBeenCalledOnce();

    const focusedSend = vi.fn();
    const focusedWindow = {
      isDestroyed: () => false,
      webContents: {
        isDestroyed: () => false,
        send: focusedSend,
        executeJavaScript: vi.fn(),
        copy: vi.fn(),
        paste: vi.fn()
      }
    };
    const mainWindow = {
      isDestroyed: () => false,
      webContents: {
        isDestroyed: () => false,
        send: vi.fn(),
        executeJavaScript: vi.fn(),
        copy: vi.fn(),
        paste: vi.fn()
      }
    };
    electronMocks.getFocusedWindow.mockReturnValue(focusedWindow);

    installMenu(() => mainWindow as never);
    const installedTemplate = electronMocks.buildFromTemplate.mock.calls.at(-1)?.[0];
    if (!installedTemplate) throw new Error('Menu template was not built');
    const newProject = submenu(topLevel(installedTemplate, 'File'))[0];
    newProject?.click?.({} as never, undefined, {} as never);

    expect(mainWindow.webContents.send).toHaveBeenCalledWith(IPC.menuCommand, 'newProject');
    expect(focusedSend).not.toHaveBeenCalled();
    expect(electronMocks.setApplicationMenu).toHaveBeenCalledOnce();
  });

  it('routes native Copy/Paste to editor commands except inside editable fields', async () => {
    const send = vi.fn();
    const webContents = {
      isDestroyed: () => false,
      send,
      executeJavaScript: vi.fn()
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(true),
      copy: vi.fn(),
      paste: vi.fn()
    };
    const focusedWindow = {
      isDestroyed: () => false,
      webContents
    };
    const editorSend = vi.fn();
    const editorWindow = {
      isDestroyed: () => false,
      webContents: {
        isDestroyed: () => false,
        send: editorSend,
        executeJavaScript: vi.fn(),
        copy: vi.fn(),
        paste: vi.fn()
      }
    };
    electronMocks.getFocusedWindow.mockReturnValue(focusedWindow);
    installMenu(() => editorWindow as never);
    const installedTemplate = electronMocks.buildFromTemplate.mock.calls.at(-1)?.[0];
    if (!installedTemplate) throw new Error('Menu template was not built');
    const editItems = submenu(topLevel(installedTemplate, 'Edit'));

    editItems.find((item) => item.id === 'copy')?.click?.({} as never, undefined, {} as never);
    await vi.waitFor(() => expect(editorSend).toHaveBeenCalledWith(IPC.menuCommand, 'copy'));
    editItems.find((item) => item.id === 'paste')?.click?.({} as never, undefined, {} as never);
    await vi.waitFor(() => expect(webContents.paste).toHaveBeenCalledOnce());

    expect(webContents.copy).not.toHaveBeenCalled();
    expect(editorSend).not.toHaveBeenCalledWith(IPC.menuCommand, 'paste');
  });
});
