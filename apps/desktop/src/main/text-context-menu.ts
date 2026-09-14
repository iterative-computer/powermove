import { BrowserWindow, Menu, shell, type ContextMenuParams, type MenuItemConstructorOptions, type WebContents } from 'electron';

/** Browser editing commands act on the focused field/frame, never layer commands. */
export function textEditMenuItems(contents: WebContents, params: ContextMenuParams, platform: NodeJS.Platform = process.platform): MenuItemConstructorOptions[] {
  const flags = params.editFlags;
  const item = (label: string, action: 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll', enabled: boolean): MenuItemConstructorOptions => ({
    label, enabled, click: () => { if (!contents.isDestroyed()) contents[action](); }
  });
  const items: MenuItemConstructorOptions[] = [];
  const selection = params.selectionText?.trim();
  // Selection labels stay compact, but search always uses the full selection.
  if (selection && params.formControlType !== 'input-password') {
    if (platform === 'darwin') {
      const label = selection.replace(/\s+/g, ' ');
      items.push({
        label: `Look Up “${label.length > 40 ? `${label.slice(0, 40)}…` : label}”`,
        click: () => { if (!contents.isDestroyed()) contents.showDefinitionForSelection(); }
      });
    }
    items.push({
      label: 'Search with Google',
      click: () => {
        const url = new URL('https://www.google.com/search');
        url.searchParams.set('q', selection);
        void shell.openExternal(url.toString()).catch(error => console.error('Could not open text search', error));
      }
    }, { type: 'separator' });
  }
  if (!params.isEditable) {
    if (params.selectionText?.length) items.push(item('Copy', 'copy', flags.canCopy));
    return items;
  }
  return [...items,
    item('Undo', 'undo', flags.canUndo), item('Redo', 'redo', flags.canRedo), { type: 'separator' },
    item('Cut', 'cut', flags.canCut), item('Copy', 'copy', flags.canCopy), item('Paste', 'paste', flags.canPaste),
    { type: 'separator' }, item('Select All', 'selectAll', flags.canSelectAll)
  ];
}
export function installTextContextMenu(contents: WebContents): void {
  contents.on('context-menu', (_event, params) => {
    const items = textEditMenuItems(contents, params);
    if (items.length) Menu.buildFromTemplate(items).popup({
      window: BrowserWindow.fromWebContents(contents) ?? undefined,
      // Enables OS-provided text services (including macOS Writing Tools).
      // https://www.electronjs.org/docs/latest/tutorial/context-menu
      frame: params.frame ?? undefined
    });
  });
}
