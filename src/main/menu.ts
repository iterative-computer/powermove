import {
  app,
  BrowserWindow,
  Menu,
  type BrowserWindow as BrowserWindowType,
  type Input,
  type MenuItemConstructorOptions
} from 'electron';

import { IPC, type MenuCommand } from '../shared/ipc';

const RENDERER_MENU_CHORDS = new Set([
  'KeyZ', 'shift+KeyZ',
  'KeyX', 'KeyC', 'KeyV', 'KeyA', 'KeyD', 'shift+KeyD', 'KeyB', 'shift+KeyH',
  'BracketRight', 'BracketLeft', 'shift+BracketRight', 'shift+BracketLeft',
  'Equal', 'shift+Equal', 'Minus', 'Digit0', 'Digit1',
]);

export function isRendererOwnedMenuInput(input: Partial<Input>): boolean {
  if ((!input.meta && !input.control) || input.alt) return false;
  const code = input.code || '';
  return RENDERER_MENU_CHORDS.has(`${input.shift ? 'shift+' : ''}${code}`);
}

/**
 * Keep displayed native menu equivalents from pre-empting the renderer's
 * field and key-repeat policy. `registerAccelerator: false` only works on
 * Windows/Linux; this focused-WebContents gate is the macOS path as well.
 */
export function installRendererMenuShortcutRouting(
  webContents: Pick<BrowserWindowType['webContents'], 'on' | 'off' | 'setIgnoreMenuShortcuts'>
): () => void {
  const route = (_event: Electron.Event, input: Input): void => {
    webContents.setIgnoreMenuShortcuts(isRendererOwnedMenuInput(input));
  };
  webContents.on('before-input-event', route);
  return () => {
    webContents.off('before-input-event', route);
    webContents.setIgnoreMenuShortcuts(false);
  };
}

const commandItem = (
  label: string,
  accelerator: string,
  command: MenuCommand,
  send: (command: MenuCommand) => void,
  registerAccelerator = true
): MenuItemConstructorOptions => ({
  id: command,
  label,
  accelerator,
  ...(registerAccelerator ? {} : { registerAccelerator: false }),
  click: () => send(command)
});

export function appMenuTemplate(
  send: (command: MenuCommand) => void,
  replayOnboarding: () => void = () => undefined
): MenuItemConstructorOptions[] {
  const viewItems: MenuItemConstructorOptions[] = [
    commandItem('Zoom In', 'CommandOrControl+=', 'zoomIn', send, false),
    commandItem('Zoom Out', 'CommandOrControl+-', 'zoomOut', send, false),
    commandItem('Actual Size', 'CommandOrControl+1', 'actualSize', send, false),
    commandItem('Fit Composition', 'CommandOrControl+0', 'fitComposition', send, false),
    commandItem('Show/Hide Layer Controls', 'CommandOrControl+Shift+H', 'toggleLayerControls', send, false),
    { type: 'separator' },
    { role: 'togglefullscreen' }
  ];
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
        {
          id: 'replayOnboarding',
          label: 'Replay Onboarding',
          accelerator: 'CommandOrControl+Shift+O',
          click: replayOnboarding
        },
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
        commandItem('Undo', 'CommandOrControl+Z', 'contextUndo', send, false),
        commandItem('Redo', 'CommandOrControl+Shift+Z', 'contextRedo', send, false),
        { type: 'separator' },
        commandItem('Cut', 'CommandOrControl+X', 'contextCut', send, false),
        commandItem('Copy', 'CommandOrControl+C', 'contextCopy', send, false),
        commandItem('Paste', 'CommandOrControl+V', 'contextPaste', send, false),
        commandItem('Select All', 'CommandOrControl+A', 'contextSelectAll', send, false),
        { type: 'separator' },
        commandItem('Duplicate Layers', 'CommandOrControl+D', 'duplicate', send, false),
        commandItem('Split at Playhead', 'CommandOrControl+Shift+D', 'split', send, false),
        { type: 'separator' },
        commandItem('Bring Forward', 'CommandOrControl+]', 'bringForward', send, false),
        commandItem('Send Backward', 'CommandOrControl+[', 'sendBackward', send, false),
        commandItem('Bring to Front', 'CommandOrControl+Shift+]', 'bringToFront', send, false),
        commandItem('Send to Back', 'CommandOrControl+Shift+[', 'sendToBack', send, false)
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

export function buildAppMenu(
  send: (command: MenuCommand) => void,
  replayOnboarding: () => void = () => undefined
): Menu {
  return Menu.buildFromTemplate(appMenuTemplate(send, replayOnboarding));
}

export function installMenu(
  getWindow: () => BrowserWindowType | null,
  replayOnboarding: () => void = () => undefined
): Menu {
  const menu = buildAppMenu((command) => {
    const focusedWindow = BrowserWindow.getFocusedWindow();
    const editorWindow = getWindow() ?? focusedWindow;
    const target = command.startsWith('context') ? (focusedWindow ?? editorWindow) : editorWindow;
    if (!target || target.isDestroyed() || target.webContents.isDestroyed()) return;
    target.webContents.send(IPC.menuCommand, command);
  }, replayOnboarding);
  Menu.setApplicationMenu(menu);
  return menu;
}
