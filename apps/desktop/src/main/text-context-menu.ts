import { BrowserWindow, Menu, shell, type ContextMenuParams, type MenuItemConstructorOptions, type WebContents } from 'electron';

type FieldHistory = { canUndo: boolean; canRedo: boolean; run: (action: 'undo' | 'redo') => void };

/** Browser editing commands act on the focused field/frame, never layer commands. */
export function textEditMenuItems(contents: WebContents, params: ContextMenuParams, platform: NodeJS.Platform = process.platform, history?: FieldHistory): MenuItemConstructorOptions[] {
  const flags = params.editFlags;
  const item = (label: string, action: 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll', enabled: boolean): MenuItemConstructorOptions => ({
    label, enabled, click: () => {
      if (contents.isDestroyed()) return;
      if (history && (action === 'undo' || action === 'redo')) history.run(action);
      else contents[action]();
    }
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
    item('Undo', 'undo', history?.canUndo ?? flags.canUndo), item('Redo', 'redo', history?.canRedo ?? flags.canRedo), { type: 'separator' },
    item('Cut', 'cut', flags.canCut), item('Copy', 'copy', flags.canCopy), item('Paste', 'paste', flags.canPaste),
    { type: 'separator' }, item('Select All', 'selectAll', flags.canSelectAll)
  ];
}
/** A row of the text menu as the renderer draws it (overlays/text-menu.ts). */
export type TextMenuRow = '-' | { id: string; label: string; enabled: boolean };

/** Spelling suggestions lead the menu, as they do in AppKit's text menu. */
export function spellingMenuItems(contents: WebContents, params: ContextMenuParams): MenuItemConstructorOptions[] {
  const word = params.misspelledWord;
  if (!params.isEditable || !word) return [];
  const suggestions = (params.dictionarySuggestions ?? []).slice(0, 5);
  return [
    ...(suggestions.length
      ? suggestions.map((suggestion): MenuItemConstructorOptions => ({ label: suggestion, click: () => { if (!contents.isDestroyed()) contents.replaceMisspelling(suggestion); } }))
      : [{ label: 'No Guesses Found', enabled: false }]),
    { label: 'Learn Spelling', click: () => { if (!contents.isDestroyed()) contents.session.addWordToSpellCheckerDictionary(word); } },
    { type: 'separator' }
  ];
}

/** Template rows → serializable rows plus the click handler for each id. */
export function textMenuRows(items: MenuItemConstructorOptions[]): { rows: TextMenuRow[]; handlers: Map<string, () => void> } {
  const rows: TextMenuRow[] = [];
  const handlers = new Map<string, () => void>();
  items.forEach((item, index) => {
    if (item.type === 'separator') { rows.push('-'); return; }
    const id = `t${index}`;
    rows.push({ id, label: item.label ?? '', enabled: item.enabled !== false });
    if (item.click) handlers.set(id, () => (item.click as () => void)());
  });
  return { rows, handlers };
}

/** Sentinel for a page that has no Powermove menu to show, such as a page still loading. */
const NO_RENDERER_MENU = '__native__';

/** Ask the page to draw the text menu in Powermove's own menu design.
    Resolves with the chosen row id, null when dismissed, or NO_RENDERER_MENU. */
async function rendererTextMenu(contents: WebContents, params: ContextMenuParams, rows: TextMenuRow[]): Promise<string | null> {
  const request = JSON.stringify({ x: params.x, y: params.y, items: rows });
  try {
    const chosen = await contents.mainFrame.executeJavaScript(
      `typeof globalThis.__pmTextMenu === 'function' ? globalThis.__pmTextMenu(${request}) : ${JSON.stringify(NO_RENDERER_MENU)}`, true);
    return typeof chosen === 'string' ? chosen : null;
  } catch {
    return NO_RENDERER_MENU;
  }
}

export function installTextContextMenu(contents: WebContents): void {
  contents.on('context-menu', async (_event, params) => {
    // The inline composer owns DOM history to preserve attachment bytes. Native
    // Chromium undo flags do not describe that history; ordinary fields still
    // use their native editing commands and the originating services frame.
    const frame = params.frame;
    let history: FieldHistory | undefined;
    if (params.isEditable && frame?.executeJavaScript) {
      try {
        const state = await frame.executeJavaScript(`(() => {
          const field = document.activeElement;
          return field?.hasAttribute('data-prompt-can-undo') ? {
            canUndo: field.dataset.promptCanUndo === 'true', canRedo: field.dataset.promptCanRedo === 'true'
          } : null;
        })()`);
        if (state && typeof state === 'object' && 'canUndo' in state && 'canRedo' in state) history = { canUndo: state.canUndo === true, canRedo: state.canRedo === true, run: action => {
          const inputType = action === 'undo' ? 'historyUndo' : 'historyRedo';
          void frame.executeJavaScript(`document.activeElement?.dispatchEvent(new InputEvent('beforeinput', {
            inputType: '${inputType}', bubbles: true, cancelable: true
          }))`).catch(() => {});
        } };
      } catch { /* A closing frame has no custom editing history to invoke. */ }
    }
    if (contents.isDestroyed()) return;
    const items = [...spellingMenuItems(contents, params), ...textEditMenuItems(contents, params, process.platform, history)];
    if (!items.length) return;
    /* Powermove draws this menu itself, like every other menu in the app. A
       page without the menu (still loading, or not an editor) gets AppKit's. */
    const { rows, handlers } = textMenuRows(items);
    const chosen = await rendererTextMenu(contents, params, rows);
    if (chosen !== NO_RENDERER_MENU) {
      if (chosen != null && !contents.isDestroyed()) handlers.get(chosen)?.();
      return;
    }
    if (contents.isDestroyed()) return;
    Menu.buildFromTemplate(items).popup({
      window: BrowserWindow.fromWebContents(contents) ?? undefined,
      // Enables OS-provided text services (including macOS Writing Tools).
      // https://www.electronjs.org/docs/latest/tutorial/context-menu
      frame: params.frame ?? undefined
    });
  });
}
