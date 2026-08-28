import {
  app,
  BrowserWindow,
  Menu,
  type BrowserWindow as BrowserWindowType,
  type MenuItemConstructorOptions
} from 'electron';

import { IPC, type MenuCommand } from '../shared/ipc';

const commandItem = (
  label: string,
  accelerator: string,
  command: MenuCommand,
  send: (command: MenuCommand) => void
): MenuItemConstructorOptions => ({
  id: command,
  label,
  accelerator,
  click: () => send(command)
});

export function appMenuTemplate(
  send: (command: MenuCommand) => void
): MenuItemConstructorOptions[] {
  const viewItems: MenuItemConstructorOptions[] = [{ role: 'togglefullscreen' }];
  if (!app.isPackaged) {
    viewItems.push(
      { type: 'separator' },
      { role: 'reload' },
      { role: 'toggleDevTools' }
    );
  }

  return [
    {
      label: app.name,
      submenu: [
        { role: 'about' },
        { type: 'separator' },
        { role: 'services' },
        { type: 'separator' },
        { role: 'hide', accelerator: 'Command+H' },
        { role: 'hideOthers', accelerator: 'Command+Alt+H' },
        { type: 'separator' },
        { role: 'quit', accelerator: 'Command+Q' }
      ]
    },
    {
      label: 'File',
      submenu: [
        commandItem('New Project', 'CommandOrControl+N', 'newProject', send),
        commandItem('Open Project…', 'CommandOrControl+O', 'open', send),
        commandItem('Save Project', 'CommandOrControl+S', 'save', send),
        commandItem('Save Project As…', 'CommandOrControl+Shift+S', 'saveAs', send),
        { type: 'separator' },
        commandItem('Export…', 'CommandOrControl+E', 'export', send)
      ]
    },
    {
      label: 'Edit',
      submenu: [
        commandItem('Undo', 'CommandOrControl+Z', 'undo', send),
        commandItem('Redo', 'CommandOrControl+Shift+Z', 'redo', send),
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        { role: 'selectAll' }
      ]
    },
    { label: 'View', submenu: viewItems },
    {
      label: 'Window',
      role: 'window',
      submenu: [
        { role: 'minimize' },
        { role: 'zoom' },
        { type: 'separator' },
        { role: 'front' }
      ]
    }
  ];
}

export function buildAppMenu(send: (command: MenuCommand) => void): Menu {
  return Menu.buildFromTemplate(appMenuTemplate(send));
}

export function installMenu(getWindow: () => BrowserWindowType | null): Menu {
  const menu = buildAppMenu((command) => {
    const window = BrowserWindow.getFocusedWindow() ?? getWindow();
    if (!window || window.isDestroyed() || window.webContents.isDestroyed()) return;
    window.webContents.send(IPC.menuCommand, command);
  });
  Menu.setApplicationMenu(menu);
  return menu;
}
