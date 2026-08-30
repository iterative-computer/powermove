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

import {
  appMenuTemplate,
  buildAppMenu,
  installMenu,
  installRendererMenuShortcutRouting,
  isRendererOwnedMenuInput,
} from './menu';
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
  it('routes renderer-owned native equivalents around the application menu', () => {
    expect(isRendererOwnedMenuInput({ meta: true, code: 'KeyB' })).toBe(true);
    expect(isRendererOwnedMenuInput({ control: true, shift: true, code: 'BracketRight' })).toBe(true);
    expect(isRendererOwnedMenuInput({ meta: true, shift: true, code: 'KeyB' })).toBe(false);
    expect(isRendererOwnedMenuInput({ meta: true, code: 'KeyQ' })).toBe(false);
    expect(isRendererOwnedMenuInput({ meta: true, alt: true, code: 'KeyB' })).toBe(false);

    let listener: ((event: never, input: any) => void) | undefined;
    const webContents = {
      on: vi.fn((_name: string, value: typeof listener) => { listener = value; }),
      off: vi.fn(),
      setIgnoreMenuShortcuts: vi.fn(),
    };
    const dispose = installRendererMenuShortcutRouting(webContents as never);
    listener?.({} as never, { meta: true, code: 'KeyB' });
    listener?.({} as never, { meta: true, code: 'KeyQ' });
    expect(webContents.setIgnoreMenuShortcuts.mock.calls).toEqual([[true], [false]]);
    dispose();
    expect(webContents.off).toHaveBeenCalledWith('before-input-event', listener);
    expect(webContents.setIgnoreMenuShortcuts).toHaveBeenLastCalledWith(false);
  });

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
    expect(undo).toMatchObject({ accelerator: 'CommandOrControl+Z' });
    expect(redo).toMatchObject({ accelerator: 'CommandOrControl+Shift+Z' });
    expect(undo?.registerAccelerator).toBe(false);
    expect(redo?.registerAccelerator).toBe(false);
    expect(undo).not.toHaveProperty('role');
    expect(redo).not.toHaveProperty('role');
    undo?.click?.({} as never, undefined, {} as never);
    redo?.click?.({} as never, undefined, {} as never);
    expect(sent).toEqual([
      'newProject', 'open', 'save', 'saveAs', 'export', 'contextUndo', 'contextRedo',
    ]);
    expect(editItems.filter((item) => item.role)).toEqual([]);
    const contextCommands = editItems.slice(3, 7);
    expect(contextCommands.map((item) => [item.label, item.accelerator, item.id])).toEqual([
      ['Cut', 'CommandOrControl+X', 'contextCut'],
      ['Copy', 'CommandOrControl+C', 'contextCopy'],
      ['Paste', 'CommandOrControl+V', 'contextPaste'],
      ['Select All', 'CommandOrControl+A', 'contextSelectAll']
    ]);
    expect(contextCommands.every((item) => item.registerAccelerator === false)).toBe(true);
    const editorCommands = editItems.filter((item) => item.click && item !== undo && item !== redo && !contextCommands.includes(item));
    expect(editorCommands.map((item) => [item.label, item.accelerator])).toEqual([
      ['Duplicate Layers', 'CommandOrControl+D'],
      ['Split at Playhead', 'CommandOrControl+B'],
      ['Hide/Show Selected Layers', 'CommandOrControl+Shift+H'],
      ['Bring Forward', 'CommandOrControl+]'],
      ['Send Backward', 'CommandOrControl+['],
      ['Bring to Front', 'CommandOrControl+Shift+]'],
      ['Send to Back', 'CommandOrControl+Shift+[']
    ]);
    expect(editorCommands.every((item) => item.registerAccelerator === false)).toBe(true);
    editorCommands.forEach((item) => item.click?.({} as never, undefined, {} as never));
    expect(sent.slice(-7)).toEqual([
      'duplicate',
      'split',
      'toggleVisibility',
      'bringForward',
      'sendBackward',
      'bringToFront',
      'sendToBack'
    ]);

    const viewItems = submenu(topLevel(template, 'View'));
    expect(viewItems.slice(0, 4).map((item) => [item.label, item.accelerator])).toEqual([
      ['Zoom In', 'CommandOrControl+='],
      ['Zoom Out', 'CommandOrControl+-'],
      ['Actual Size', 'CommandOrControl+1'],
      ['Fit Composition', 'CommandOrControl+0']
    ]);
    expect(viewItems.slice(0, 4).every((item) => item.registerAccelerator === false)).toBe(true);

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
    expect(packaged.map((item) => item.label ?? item.type ?? item.role)).toEqual([
      'Zoom In',
      'Zoom Out',
      'Actual Size',
      'Fit Composition',
      'separator',
      'togglefullscreen'
    ]);
  });

  it('builds and installs the menu, targeting the focused window before the main window', () => {
    buildAppMenu(() => undefined);
    expect(electronMocks.buildFromTemplate).toHaveBeenCalledOnce();

    const focusedSend = vi.fn();
    const focusedWindow = {
      isDestroyed: () => false,
      webContents: { isDestroyed: () => false, send: focusedSend }
    };
    const mainWindow = {
      isDestroyed: () => false,
      webContents: { isDestroyed: () => false, send: vi.fn() }
    };
    electronMocks.getFocusedWindow.mockReturnValue(focusedWindow);

    installMenu(() => mainWindow as never);
    const installedTemplate = electronMocks.buildFromTemplate.mock.calls.at(-1)?.[0];
    if (!installedTemplate) throw new Error('Menu template was not built');
    const newProject = submenu(topLevel(installedTemplate, 'File'))[0];
    newProject?.click?.({} as never, undefined, {} as never);

    expect(focusedSend).toHaveBeenCalledWith(IPC.menuCommand, 'newProject');
    expect(mainWindow.webContents.send).not.toHaveBeenCalled();
    expect(electronMocks.setApplicationMenu).toHaveBeenCalledOnce();
  });
});
