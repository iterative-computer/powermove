// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { InlinePrompt, displayPromptText } from './inline-prompt';
const file = { id: 'file', name: 'reference.png', type: 'image/png', size: 1, promptOffset: 6 };
function setup(text = 'hello world') {
  const element = document.createElement('div'); element.contentEditable = 'true'; document.body.append(element);
  const change = vi.fn(); const editor = new InlinePrompt(element, change); editor.sync(text, [file], true);
  return { element, editor, change };
}
function caretAfter(node: Node) { const range = document.createRange(); range.setStartAfter(node); range.collapse(true); window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); }
function key(editor: InlinePrompt, name: string, options = {}) { return editor.keydown(new KeyboardEvent('keydown', { key: name, cancelable: true, ...options })); }
afterEach(() => { document.body.replaceChildren(); window.getSelection()?.removeAllRanges(); });
it('keeps attachments between surrounding text and includes their position in the request', () => {
  const { editor, element } = setup();
  expect(element.childNodes[0]!.textContent).toBe('hello ');
  expect(element.childNodes[2]!.textContent).toBe('world');
  expect(editor.requestText()).toBe('hello [Attachment: reference.png]world');
  expect(editor.read()).toEqual({ text: 'hello world', attachments: [file] });
});
it('selects on first Backspace, ignores a held key, and removes on the second press', () => {
  const { editor, element } = setup();
  caretAfter(element.querySelector('[data-attachment-id]')!);
  expect(key(editor, 'Backspace')).toBe(true);
  expect(element.querySelector('.is-selected')).not.toBeNull();
  key(editor, 'Backspace', { repeat: true });
  expect(editor.read().attachments).toHaveLength(1);
  key(editor, 'Backspace');
  expect(editor.read()).toEqual({ text: 'hello world', attachments: [] });
  key(editor, 'z', { metaKey: true });
  expect(editor.read().attachments).toEqual([file]);
  key(editor, 'z', { metaKey: true, shiftKey: true });
  expect(editor.read().attachments).toHaveLength(0);
});
it('inserts newly read files at the bookmarked caret and retains their bytes', () => {
  const { editor, element } = setup();
  const range = document.createRange(); range.setStart(element.firstChild!, 2); range.collapse(true);
  window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); editor.remember();
  editor.sync('hello world', [file, { id: 'other', name: 'b.aep', type: 'application/octet-stream', size: 3, dataBase64: 'YWJj' }]);
  expect(editor.read().attachments.map(item => [item.id, item.promptOffset])).toEqual([['other', 2], ['file', 6]]);
  expect(editor.read().attachments[0]!.dataBase64).toBe('YWJj');
});
it('does not lose a selected attachment when pasting text', () => {
  const { editor, element } = setup(); caretAfter(element.querySelector('[data-attachment-id]')!);
  key(editor, 'Backspace'); editor.pasteText('new ');
  expect(editor.read().attachments).toHaveLength(1);
  expect(editor.read().text).toBe('hello new world');
});
it('clears undo history when switching conversations', () => {
  const { editor } = setup(); editor.setText('changed'); editor.sync('second', [], true); key(editor, 'z', { metaKey: true });
  expect(editor.read()).toEqual({ text: 'second', attachments: [] });
});
it('cuts and pastes an attachment as an intact token, with a single undo', () => {
  const { editor, element } = setup(); caretAfter(element.querySelector('[data-attachment-id]')!); key(editor, 'Backspace');
  const clipboard = new Map<string, string>();
  const event = { preventDefault: vi.fn(), clipboardData: { setData: (type: string, value: string) => clipboard.set(type, value), getData: (type: string) => clipboard.get(type) || '' } } as any;
  editor.clipboard(event, true);
  expect(editor.read().attachments).toHaveLength(0);
  expect(clipboard.get('text/plain')).toBe('reference.png');
  const range = document.createRange(); range.setStart(element.firstChild!, 0); range.collapse(true); window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
  expect(editor.paste(event)).toBe(true);
  expect(editor.read().attachments[0]!.promptOffset).toBe(0);
  key(editor, 'z', { metaKey: true }); expect(editor.read().attachments).toHaveLength(0);
});
it('does not serialize large file contents or rebuild a token when synchronizing unchanged input', () => {
  const { editor, element } = setup();
  const attachment = { ...file, dataBase64: 'abcd'.repeat(1_000_000) };
  editor.sync('hello world', [attachment], true);
  const token = element.querySelector('[data-attachment-id]');
  const stringify = vi.spyOn(JSON, 'stringify');
  for (let i = 0; i < 50; i++) editor.sync('hello world', [attachment]);
  expect(stringify).not.toHaveBeenCalled(); stringify.mockRestore();
  expect(element.querySelector('[data-attachment-id]')).toBe(token);
});
it('highlights attachments within native text selections and only then allows moving them', () => {
  const { editor, element } = setup();
  const token = element.querySelector<HTMLElement>('[data-attachment-id]')!;
  expect(token.draggable).toBe(false);
  const range = document.createRange();
  range.setStart(element.firstChild!, 3); range.setEnd(element.lastChild!, 2);
  window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
  editor.remember();
  expect(token.classList.contains('is-range-selected')).toBe(true);
  expect(token.draggable).toBe(true);
  caretAfter(token); editor.remember();
  expect(token.classList.contains('is-range-selected')).toBe(false);
  expect(token.draggable).toBe(false);
  expect(editor.read().attachments).toEqual([file]);
});
it('deletes text before a file without letting Chromium insert a placeholder line break', () => {
  const { editor, element } = setup();
  const text = element.firstChild!;
  const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, 6);
  window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
  const event = new InputEvent('beforeinput', { inputType: 'deleteContentBackward', cancelable: true });
  Object.defineProperty(event, 'getTargetRanges', { value: () => [range] });
  editor.beforeinput(event);
  expect(event.defaultPrevented).toBe(true);
  expect(element.querySelector('br')).toBeNull();
  expect(editor.read().text).toBe('world');
  expect(editor.read().attachments[0]!.promptOffset).toBe(0);
});
it('provides caret positions on both sides of a file-only draft without adding submitted characters', () => {
  const { editor, element } = setup();
  editor.sync('', [{ ...file, promptOffset: 0 }], true);
  const token = element.querySelector('[data-attachment-id]')!;
  expect(token.previousSibling?.nodeType).toBe(Node.TEXT_NODE);
  expect(token.nextSibling?.nodeType).toBe(Node.TEXT_NODE);
  const range = document.createRange(); range.setStart(token.previousSibling!, 0); range.collapse(true);
  window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
  editor.beforeinput(new InputEvent('beforeinput', { inputType: 'insertText', data: 'before ', cancelable: true }));
  caretAfter(token);
  editor.beforeinput(new InputEvent('beforeinput', { inputType: 'insertText', data: ' after', cancelable: true }));
  expect(editor.requestText()).toBe('before [Attachment: reference.png] after');
  expect(element.querySelector('br')).toBeNull();
});

it('publishes custom undo and redo availability for native editing menus', () => {
  const { editor, element } = setup();
  expect(element.dataset.promptCanUndo).toBe('false');
  expect(element.dataset.promptCanRedo).toBe('false');
  editor.setText('changed');
  expect(element.dataset.promptCanUndo).toBe('true');
  editor.beforeinput(new InputEvent('beforeinput', { inputType: 'historyUndo', cancelable: true }));
  expect(editor.read().text).toBe('hello world');
  expect(element.dataset.promptCanUndo).toBe('false');
  expect(element.dataset.promptCanRedo).toBe('true');
  editor.beforeinput(new InputEvent('beforeinput', { inputType: 'historyRedo', cancelable: true }));
  expect(editor.read().text).toBe('changed');
  expect(element.dataset.promptCanRedo).toBe('false');
  editor.sync('new thread', [], true);
  expect(element.dataset.promptCanUndo).toBe('false');
});

it('removes invisible caret stops once typing supplies real text beside a token', () => {
  const { editor, element } = setup('hello ');
  const token = element.querySelector('[data-attachment-id]')!;
  caretAfter(token);
  editor.pasteText(' suffix', true);
  expect(editor.read().text).toBe('hello  suffix');
  expect(token.nextSibling!.textContent).toBe(' suffix');
  const selection = window.getSelection()!;
  expect(selection.anchorNode).toBe(token.nextSibling);
  expect(selection.anchorOffset).toBe(' suffix'.length);
});

it('drops the model-facing attachment markers from what the transcript shows', () => {
  expect(displayPromptText('[Attachment: image.png]now it looks like this')).toBe('now it looks like this');
  expect(displayPromptText('before [Attachment: a.png] after')).toBe('before after');
  expect(displayPromptText('hello [Attachment: a.png]world')).toBe('hello world');
  expect(displayPromptText('[Attachment: a.png]')).toBe('');
});
