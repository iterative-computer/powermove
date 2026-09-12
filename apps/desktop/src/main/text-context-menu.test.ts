import { describe, expect, it, vi } from 'vitest';
vi.mock('electron', () => ({ Menu: { buildFromTemplate: vi.fn() } }));
import { textEditMenuItems } from './text-context-menu';
const flags = { canUndo: true, canRedo: false, canCut: true, canCopy: true, canPaste: true, canSelectAll: true };
describe('native text context menus', () => {
  it('uses browser editing actions and honors availability', () => {
    const contents: any = { isDestroyed: () => false, copy: vi.fn(), paste: vi.fn(), undo: vi.fn() };
    const items = textEditMenuItems(contents, { isEditable: true, editFlags: flags } as any);
    expect(items.find(i => i.label === 'Redo')?.enabled).toBe(false);
    for (const label of ['Copy', 'Paste', 'Undo']) items.find(i => i.label === label)?.click?.({} as never, undefined, {} as never);
    expect(contents.copy).toHaveBeenCalledOnce(); expect(contents.paste).toHaveBeenCalledOnce(); expect(contents.undo).toHaveBeenCalledOnce();
  });
  it('offers only copy for read-only selection, and leaves canvas menus alone', () => {
    const contents: any = {};
    expect(textEditMenuItems(contents, { isEditable: false, selectionText: 'Reply', editFlags: flags } as any).map(i => i.label)).toEqual(['Copy']);
    expect(textEditMenuItems(contents, { isEditable: false, selectionText: '', editFlags: flags } as any)).toEqual([]);
  });
});
