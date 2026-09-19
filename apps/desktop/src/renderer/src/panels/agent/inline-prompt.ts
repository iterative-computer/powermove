import type { PromptAttachment } from './attachments';

export type InlineAttachment = PromptAttachment & { promptOffset?: number };
type Draft = { text: string; attachments: InlineAttachment[] };

/** Owns the editable DOM so reactive renders never interrupt the caret or IME. */
export class InlinePrompt {
  private items = new Map<string, InlineAttachment>();
  private armed: HTMLElement | null = null;
  private bookmark: Range | null = null;
  private dragging: HTMLElement | null = null;
  private history: Draft[] = [];
  private historyIndex = -1;
  private lastEdit = 0;
  constructor(readonly element: HTMLElement, private change: (draft: Draft) => void) {}

  read(): Draft {
    let text = '';
    const attachments: InlineAttachment[] = [];
    const walk = (node: Node) => {
      if (node instanceof HTMLElement && node.hasAttribute('data-prompt-tail')) return;
      if (node instanceof HTMLElement && node.dataset.attachmentId) {
        const item = this.items.get(node.dataset.attachmentId);
        if (item) attachments.push({ ...item, promptOffset: text.length });
      } else if (node.nodeType === Node.TEXT_NODE) text += (node.textContent || '').replace(/\u200b/g, '');
      else if (node instanceof HTMLBRElement) text += '\n';
      else {
        if (node !== this.element && node instanceof HTMLElement && /^(DIV|P)$/.test(node.tagName) && text && !text.endsWith('\n')) text += '\n';
        node.childNodes.forEach(walk);
      }
    };
    walk(this.element);
    return { text, attachments };
  }

  requestText(): string {
    const draft = this.read();
    let result = '', cursor = 0;
    for (const item of draft.attachments) {
      const offset = item.promptOffset ?? draft.text.length;
      result += draft.text.slice(cursor, offset) + `[Attachment: ${item.name}]`;
      cursor = offset;
    }
    return result + draft.text.slice(cursor);
  }

  remember = () => {
    const selection = window.getSelection();
    const range = selection?.rangeCount ? selection.getRangeAt(0) : null;
    if (range && this.element.contains(selection!.anchorNode)) this.bookmark = range.cloneRange();
    this.element.querySelectorAll<HTMLElement>('[data-attachment-id]').forEach(token => {
      const selected = !!range && !range.collapsed && this.element.contains(range.commonAncestorContainer) && range.intersectsNode(token);
      token.classList.toggle('is-range-selected', selected);
      // Unselected files behave like text; select one before dragging it to move.
      token.draggable = selected;
    });
  };

  private token(item: InlineAttachment): HTMLElement {
    this.items.set(item.id, item);
    const token = document.createElement('span');
    token.className = 'agent-inline-attachment';
    token.contentEditable = 'false';
    token.draggable = false;
    token.dataset.attachmentId = item.id;
    token.setAttribute('aria-label', `Attachment: ${item.name}`);
    token.title = `${item.name} · Select, then drag to move · Backspace twice to remove`;
    if (item.dataUrl) {
      const image = document.createElement('img'); image.src = item.dataUrl; image.alt = ''; image.draggable = false; token.append(image);
    } else {
      const badge = document.createElement('span'); badge.className = 'agent-inline-attachment-type';
      badge.textContent = (item.name.split('.').pop() || 'file').slice(0, 5).toUpperCase();
      badge.setAttribute('aria-hidden', 'true'); token.append(badge);
    }
    const label = document.createElement('span'); label.textContent = item.name.replace(/\.[^.]+$/, '') || item.name; token.append(label);
    return token;
  }

  sync(text: string, attachments: InlineAttachment[], reset = false) {
    const current = this.read();
    if (!reset && current.text === text && current.attachments.length === attachments.length && current.attachments.every((item, index) => item.id === attachments[index]?.id && item.promptOffset === (attachments[index]?.promptOffset ?? text.length))) return;
    const existing = new Set(current.attachments.map(item => item.id));
    const added = attachments.filter(item => !existing.has(item.id));
    // New picker/paste attachments belong at the saved caret, even after an async read.
    if (!reset && current.text === text && added.length && attachments.every(item => existing.has(item.id) || item.promptOffset === undefined)) {
      const wanted = new Set(attachments.map(item => item.id));
      this.element.querySelectorAll<HTMLElement>('[data-attachment-id]').forEach(node => { if (!wanted.has(node.dataset.attachmentId!)) node.remove(); });
      const range = this.bookmark && this.element.contains(this.bookmark.startContainer) ? this.bookmark.cloneRange() : document.createRange();
      if (!this.element.contains(range.startContainer)) { range.selectNodeContents(this.element); range.collapse(false); }
      for (const item of added) { const token = this.token(item); range.insertNode(token); range.setStartAfter(token); range.collapse(true); }
      this.select(range); this.commit(false); return;
    }
    this.items.clear();
    this.render({ text, attachments });
    this.history = [this.read()]; this.historyIndex = 0;
  }

  private render(draft: Draft) {
    this.disarm(); this.element.replaceChildren(); this.bookmark = null;
    let cursor = 0;
    for (const item of [...draft.attachments].sort((a, b) => (a.promptOffset ?? draft.text.length) - (b.promptOffset ?? draft.text.length))) {
      const offset = Math.max(cursor, Math.min(draft.text.length, item.promptOffset ?? draft.text.length));
      this.element.append(document.createTextNode(draft.text.slice(cursor, offset)), this.token(item)); cursor = offset;
    }
    this.element.append(document.createTextNode(draft.text.slice(cursor)));
    this.trailingLine(draft.text);
    this.caretStops();
  }

  private caretStops() {
    // Chromium needs real text on both sides of a noneditable inline node to
    // place a caret there without inventing a new line. These invisible stops
    // are presentation-only and never enter the draft or submitted message.
    this.element.childNodes.forEach(node => {
      if (node.nodeType !== Node.TEXT_NODE || !(node.textContent || '').replace(/\u200b/g, '')) return;
      const text = node as Text;
      for (let index = text.length - 1; index >= 0; index--) if (text.data[index] === '\u200b') text.deleteData(index, 1);
    });
    this.element.querySelectorAll<HTMLElement>('[data-attachment-id]').forEach(token => {
      if (token.previousSibling?.nodeType !== Node.TEXT_NODE) token.before(document.createTextNode('\u200b'));
      else if (!token.previousSibling.textContent) token.previousSibling.textContent = '\u200b';
      if (token.nextSibling?.nodeType !== Node.TEXT_NODE) token.after(document.createTextNode('\u200b'));
      else if (!token.nextSibling.textContent) token.nextSibling.textContent = '\u200b';
    });
  }

  private trailingLine(text: string) {
    const tail = this.element.querySelector('[data-prompt-tail]');
    if (text.endsWith('\n')) {
      if (!tail) { const br = document.createElement('br'); br.setAttribute('data-prompt-tail', ''); this.element.append(br); }
    } else tail?.remove();
  }

  private select(range: Range) { const selection = window.getSelection(); selection?.removeAllRanges(); selection?.addRange(range); this.bookmark = range.cloneRange(); }
  private disarm() { this.armed?.classList.remove('is-selected'); this.armed = null; }
  private commit(typing: boolean) {
    this.element.normalize();
    this.caretStops();
    const draft = this.read();
    this.trailingLine(draft.text);
    this.history = this.history.slice(0, this.historyIndex + 1);
    const now = Date.now();
    if (typing && now - this.lastEdit < 600 && this.historyIndex > 0) this.history[this.historyIndex] = draft;
    else { this.history.push(draft); this.historyIndex++; }
    if (this.history.length > 100) { this.history.shift(); this.historyIndex--; }
    this.lastEdit = typing ? now : 0;
    this.change(draft); this.remember();
  }
  input = () => { this.disarm(); this.commit(true); };
  setText(text: string) { this.render({ text, attachments: this.read().attachments }); this.commit(false); }
  selectAll() { const range = document.createRange(); range.selectNodeContents(this.element); this.select(range); }

  keydown(event: KeyboardEvent): boolean {
    if (event.isComposing) return false;
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z') {
      event.preventDefault();
      const index = this.historyIndex + (event.shiftKey ? 1 : -1);
      if (index >= 0 && index < this.history.length) {
        this.historyIndex = index; this.render(this.history[index]!); this.change(this.read());
        const range = document.createRange(); range.selectNodeContents(this.element); range.collapse(false); this.select(range);
      }
      return true;
    }
    if (event.key !== 'Backspace' && event.key !== 'Delete') { this.disarm(); return false; }
    const selection = window.getSelection();
    if (!selection?.rangeCount || !this.element.contains(selection.anchorNode)) return false;
    const range = selection.getRangeAt(0);
    let candidates = [...this.element.querySelectorAll<HTMLElement>('[data-attachment-id]')].filter(token => range.intersectsNode(token));
    if (range.collapsed) {
      const probe = range.cloneRange(); probe.selectNodeContents(this.element);
      if (event.key === 'Backspace') probe.setEnd(range.startContainer, range.startOffset);
      else probe.setStart(range.startContainer, range.startOffset);
      const fragment = probe.cloneContents();
      const nodes = [...fragment.childNodes];
      if (event.key === 'Backspace') nodes.reverse();
      candidates = [];
      for (const node of nodes) {
        if (node.nodeType === Node.TEXT_NODE && !(node.textContent || '').replace(/\u200b/g, '')) continue;
        if (node instanceof HTMLElement && node.dataset.attachmentId) {
          const token = [...this.element.querySelectorAll<HTMLElement>('[data-attachment-id]')].find(item => item.dataset.attachmentId === node.dataset.attachmentId);
          if (token) candidates = [token];
        }
        break;
      }
    }
    if (!candidates.length) { this.disarm(); return false; }
    event.preventDefault();
    if (event.repeat) return true;
    const token = candidates[0]!;
    if (this.armed === token) {
      const deletion = window.getSelection()!.getRangeAt(0); deletion.deleteContents(); this.disarm(); this.commit(false);
    } else {
      this.disarm(); this.armed = token; token.classList.add('is-selected');
      if (range.collapsed) { range.selectNode(token); this.select(range); }
    }
    return true;
  }

  beforeinput = (event: InputEvent) => {
    if (event.inputType === 'historyUndo' || event.inputType === 'historyRedo') {
      event.preventDefault(); this.keydown(new KeyboardEvent('keydown', { key: 'z', metaKey: true, shiftKey: event.inputType === 'historyRedo' })); return;
    }
    if (event.inputType === 'insertText' && event.data !== null && !event.isComposing) {
      event.preventDefault(); this.pasteText(event.data, true); return;
    }
    if (event.inputType.startsWith('insert')) {
      const selection = window.getSelection();
      if (selection?.rangeCount && !selection.isCollapsed) {
        const range = selection.getRangeAt(0);
        if ([...this.element.querySelectorAll('[data-attachment-id]')].some(token => range.intersectsNode(token))) {
          range.collapse(false); this.select(range); this.disarm();
        }
      }
    }
    if (event.inputType === 'deleteContentBackward' || event.inputType === 'deleteContentForward') {
      const key = new KeyboardEvent('keydown', { key: event.inputType === 'deleteContentBackward' ? 'Backspace' : 'Delete', cancelable: true });
      if (this.keydown(key)) { event.preventDefault(); return; }
      // Chromium inserts a placeholder <br> when native deletion empties the
      // text before a noneditable token. Apply its exact deletion range ourselves
      // so a file-only line does not acquire an unintended blank line above it.
      const target = event.getTargetRanges?.()[0];
      if (target && this.element.querySelector('[data-attachment-id]')) {
        const deletion = document.createRange();
        deletion.setStart(target.startContainer, target.startOffset);
        deletion.setEnd(target.endContainer, target.endOffset);
        if (!deletion.collapsed && this.element.contains(deletion.commonAncestorContainer)
          && ![...this.element.querySelectorAll('[data-attachment-id]')].some(token => deletion.intersectsNode(token))) {
          event.preventDefault(); deletion.deleteContents(); deletion.collapse(true);
          this.select(deletion); this.commit(false);
        }
      }
    }
  };
  pointerdown = () => { this.disarm(); };
  clipboard(event: ClipboardEvent, cut: boolean) {
    const selection = window.getSelection();
    if (!event.clipboardData || !selection?.rangeCount || selection.isCollapsed) return;
    const range = selection.getRangeAt(0);
    if (!this.element.contains(range.commonAncestorContainer)) return;
    const fragment = range.cloneContents();
    const parts: Array<{ text: string } | { id: string }> = [];
    const walk = (node: Node) => {
      if (node instanceof HTMLElement && node.hasAttribute('data-prompt-tail')) return;
      if (node instanceof HTMLElement && node.dataset.attachmentId) parts.push({ id: node.dataset.attachmentId });
      else if (node.nodeType === Node.TEXT_NODE) parts.push({ text: (node.textContent || '').replace(/\u200b/g, '') });
      else if (node instanceof HTMLBRElement) parts.push({ text: '\n' });
      else node.childNodes.forEach(walk);
    };
    walk(fragment);
    event.preventDefault();
    event.clipboardData.setData('text/plain', parts.map(part => 'text' in part ? part.text : this.items.get(part.id)?.name || '').join(''));
    event.clipboardData.setData('application/x-powermove-prompt', JSON.stringify(parts));
    if (cut) { range.deleteContents(); this.disarm(); this.commit(false); }
  }
  paste(event: ClipboardEvent): boolean {
    const serialized = event.clipboardData?.getData('application/x-powermove-prompt');
    if (!serialized) return false;
    let parts: Array<{ text?: string; id?: string }>;
    try { parts = JSON.parse(serialized); } catch { return false; }
    if (!Array.isArray(parts) || parts.length > 1000 || parts.some(part => !part || (typeof part.text !== 'string' && (typeof part.id !== 'string' || !this.items.has(part.id))))) return false;
    event.preventDefault();
    const selection = window.getSelection();
    const range = selection?.rangeCount && this.element.contains(selection.anchorNode) ? selection.getRangeAt(0) : document.createRange();
    if (!this.element.contains(range.startContainer)) { range.selectNodeContents(this.element); range.collapse(false); }
    if (!range.collapsed && [...this.element.querySelectorAll('[data-attachment-id]')].some(token => range.intersectsNode(token))) range.collapse(false);
    range.deleteContents();
    const fragment = document.createDocumentFragment(), moved = new Set<string>();
    for (const part of parts) {
      if (typeof part.text === 'string') fragment.append(document.createTextNode(part.text));
      else if (!moved.has(part.id!)) {
        const item = this.items.get(part.id!)!; moved.add(item.id);
        this.element.querySelectorAll<HTMLElement>('[data-attachment-id]').forEach(token => { if (token.dataset.attachmentId === item.id) token.remove(); });
        fragment.append(this.token(item));
      }
    }
    const last = fragment.lastChild;
    range.insertNode(fragment);
    if (last) range.setStartAfter(last);
    range.collapse(true); this.select(range); this.commit(false);
    return true;
  }
  pasteText(text: string, typing = false) {
    const selection = window.getSelection();
    let range = selection?.rangeCount && this.element.contains(selection.anchorNode) ? selection.getRangeAt(0) : null;
    if (!range) { range = document.createRange(); range.selectNodeContents(this.element); range.collapse(false); }
    // Pasting text cannot silently delete a selected attachment.
    if ([...this.element.querySelectorAll('[data-attachment-id]')].some(token => range!.intersectsNode(token)) && !range.collapsed) range.collapse(false);
    range.deleteContents(); const node = document.createTextNode(text); range.insertNode(node); range.setStart(node, node.length); range.collapse(true); this.trailingLine(this.read().text); this.select(range); this.commit(typing);
  }
  dragstart = (event: DragEvent) => {
    this.dragging = (event.target as HTMLElement).closest('[data-attachment-id]');
    if (this.dragging && event.dataTransfer) { event.dataTransfer.setData('application/x-powermove-attachment', this.dragging.dataset.attachmentId!); event.dataTransfer.effectAllowed = 'move'; }
  };
  drop(event: DragEvent): boolean {
    if (!this.dragging) return false;
    event.preventDefault(); event.stopPropagation();
    const range = document.caretRangeFromPoint(event.clientX, event.clientY);
    if (range && this.element.contains(range.startContainer) && !this.dragging.contains(range.startContainer)) {
      range.insertNode(this.dragging); range.setStartAfter(this.dragging); range.collapse(true); this.select(range); this.commit(false);
    }
    this.dragging = null; return true;
  }
  dragend = () => { this.dragging = null; };
}
