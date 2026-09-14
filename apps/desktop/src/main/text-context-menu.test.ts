import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ContextMenuParams, WebContents } from 'electron';

vi.mock('electron', () => ({
  BrowserWindow: { fromWebContents: vi.fn() },
  Menu: { buildFromTemplate: vi.fn() },
  shell: { openExternal: vi.fn().mockResolvedValue(undefined) }
}));
import { BrowserWindow, Menu, shell } from 'electron';
import { installTextContextMenu, textEditMenuItems } from './text-context-menu';

const flags = { canUndo: true, canRedo: false, canCut: true, canCopy: true, canPaste: true, canSelectAll: true };
const params = (overrides: Partial<ContextMenuParams> = {}) => ({
  isEditable: true, selectionText: 'Native controls', editFlags: flags, ...overrides
}) as ContextMenuParams;
const contents = () => ({
  isDestroyed: vi.fn(() => false), copy: vi.fn(), paste: vi.fn(), undo: vi.fn(),
  cut: vi.fn(), redo: vi.fn(), selectAll: vi.fn(), showDefinitionForSelection: vi.fn(), on: vi.fn()
}) as unknown as WebContents;

beforeEach(() => vi.clearAllMocks());

describe('native text context menus', () => {
  it('uses browser editing actions and honors availability', () => {
    const target = contents();
    const items = textEditMenuItems(target, params());
    expect(items.find(i => i.label === 'Redo')?.enabled).toBe(false);
    for (const label of ['Copy', 'Paste', 'Undo', 'Cut', 'Select All']) {
      items.find(i => i.label === label)?.click?.({} as never, undefined, {} as never);
    }
    for (const action of ['copy', 'paste', 'undo', 'cut', 'selectAll'] as const) expect(target[action]).toHaveBeenCalledOnce();
  });

  it('offers lookup, search and copy for read-only selection without editing actions', () => {
    expect(textEditMenuItems(contents(), params({ isEditable: false }), 'darwin').filter(i => i.type !== 'separator').map(i => i.label))
      .toEqual(['Look Up “Native controls”', 'Search with Google', 'Copy']);
    expect(textEditMenuItems(contents(), params({ isEditable: false, selectionText: '' }))).toEqual([]);
  });

  it('opens the native definition and searches the full selection with an encoded query', () => {
    const target = contents();
    const selection = 'A long selection & another phrase?\n'.repeat(3).trim();
    const items = textEditMenuItems(target, params({ selectionText: selection }), 'darwin');
    expect(items[0]!.label).toMatch(/^Look Up “.{40}…”$/);
    items[0]!.click?.({} as never, undefined, {} as never);
    items[1]!.click?.({} as never, undefined, {} as never);
    expect(target.showDefinitionForSelection).toHaveBeenCalledOnce();
    const url = new URL(vi.mocked(shell.openExternal).mock.calls[0]![0]);
    expect(url.origin + url.pathname).toBe('https://www.google.com/search');
    expect([...url.searchParams]).toEqual([['q', selection]]);
  });

  it('omits selection services for empty, whitespace-only and password selections', () => {
    for (const context of [params({ selectionText: '' }), params({ selectionText: ' \n ' }), params({ formControlType: 'input-password' })]) {
      expect(textEditMenuItems(contents(), context, 'darwin').some(i => i.label?.startsWith('Look Up') || i.label === 'Search with Google')).toBe(false);
    }
  });

  it('does not offer macOS lookup on other platforms', () => {
    const items = textEditMenuItems(contents(), params(), 'win32');
    expect(items.some(i => i.label?.startsWith('Look Up'))).toBe(false);
    expect(items.some(i => i.label === 'Search with Google')).toBe(true);
  });

  it('ignores editing and definition actions after their window closes', () => {
    const target = contents();
    const items = textEditMenuItems(target, params(), 'darwin');
    vi.mocked(target.isDestroyed).mockReturnValue(true);
    for (const label of ['Copy', 'Look Up “Native controls”']) items.find(i => i.label === label)?.click?.({} as never, undefined, {} as never);
    expect(target.copy).not.toHaveBeenCalled();
    expect(target.showDefinitionForSelection).not.toHaveBeenCalled();
  });

  it('associates the popup with the originating frame for native text services', () => {
    const target = contents();
    const popup = vi.fn();
    vi.mocked(Menu.buildFromTemplate).mockReturnValue({ popup } as any);
    vi.mocked(BrowserWindow.fromWebContents).mockReturnValue({ id: 1 } as any);
    installTextContextMenu(target);
    const handler = vi.mocked(target.on).mock.calls[0]![1] as Function;
    const frame = { routingId: 42 } as any;
    handler({}, params({ frame }));
    expect(popup).toHaveBeenCalledWith({ window: { id: 1 }, frame });
    handler({}, params({ isEditable: false, selectionText: '' }));
    expect(popup).toHaveBeenCalledOnce();
  });
});
