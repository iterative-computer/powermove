import { BrowserWindow, Menu, type ContextMenuParams, type MenuItemConstructorOptions, type WebContents } from 'electron';

/** Browser editing commands act on the focused field/frame, never layer commands. */
export function textEditMenuItems(contents: WebContents, params: ContextMenuParams): MenuItemConstructorOptions[] {
  const flags = params.editFlags;
  const item = (label: string, action: 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll', enabled: boolean): MenuItemConstructorOptions => ({
    label, enabled, click: () => { if (!contents.isDestroyed()) contents[action](); }
  });
  if (!params.isEditable) return params.selectionText?.length ? [item('Copy', 'copy', flags.canCopy)] : [];
  return [
    item('Undo', 'undo', flags.canUndo), item('Redo', 'redo', flags.canRedo), { type: 'separator' },
    item('Cut', 'cut', flags.canCut), item('Copy', 'copy', flags.canCopy), item('Paste', 'paste', flags.canPaste),
    { type: 'separator' }, item('Select All', 'selectAll', flags.canSelectAll)
  ];
}
export function installTextContextMenu(contents: WebContents): void {
  contents.on('context-menu', (_event, params) => {
    const items = textEditMenuItems(contents, params);
    if (items.length) Menu.buildFromTemplate(items).popup({ window: BrowserWindow.fromWebContents(contents) ?? undefined });
  });
}
