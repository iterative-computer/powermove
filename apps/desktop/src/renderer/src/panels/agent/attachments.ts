import { bridge } from '../../kernel/bridge';
export const ATTACHMENT_HINT = 'Attach images or files';

export interface PromptAttachment {
  id: string;
  name: string;
  type: string;
  size: number;
  /** Text offset of the inline token in the saved composer draft. */
  promptOffset?: number;
  dataUrl?: string;
  dataBase64?: string;
  content?: string;
  /** Native path captured from the File at picker/drop time. */
  sourcePath?: string;
}

export async function readPromptAttachment(file: File, id: string): Promise<PromptAttachment> {
  const imageTypes: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif', svg: 'image/svg+xml', avif: 'image/avif', bmp: 'image/bmp' };
  const extension = file.name.split('.').pop()?.toLowerCase() || '';
  const type = file.type || (Object.hasOwn(imageTypes, extension) ? imageTypes[extension]! : 'application/octet-stream');
  const image = isPreviewableImageType(type);
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error(`Could not read ${file.name}.`));
    reader.onabort = () => reject(new Error(`Reading ${file.name} was cancelled.`));
    reader.readAsDataURL(file);
  });
  const item: PromptAttachment = { id, name: file.name || 'Attachment', type, size: file.size };
  let sourcePath: string | null = null;
  try { sourcePath = bridge()?.media?.sourcePath?.(file) || null; } catch { /* Clipboard-backed Files have no native path. */ }
  if (sourcePath) item.sourcePath = sourcePath;
  if (image) item.dataUrl = dataUrl.replace(/^data:[^;]*;/, `data:${type};`);
  if (!isAgentImageType(type)) {
    // Preserve binary bytes, including zero-byte files. Never attach just a filename.
    item.dataBase64 = dataUrl.slice(dataUrl.indexOf(',') + 1);
    if (type.startsWith('text/') || /\.(txt|md|json|js|ts|tsx|jsx|css|html?|svg|xml|csv|log)$/i.test(file.name)) {
      item.content = await file.text();
    }
  }
  return item;
}

export function isPreviewableImageType(type: unknown): boolean {
  return typeof type === 'string' && /^image\/(?:png|jpeg|webp|gif|svg\+xml|avif|bmp)$/i.test(type);
}

export function isAgentImageType(type: unknown): boolean {
  return typeof type === 'string' && /^image\/(?:png|jpeg|webp|gif)$/i.test(type);
}

export function isAgentImageAttachment(item: Record<string, any>): boolean {
  return typeof item.dataUrl === 'string' && isAgentImageType(item.type);
}

export function openImagePreview(PM: Record<string, any>, name: string, src: string): void {
  const body = document.createElement('div');
  body.className = 'agent-image-preview';
  const image = document.createElement('img');
  image.src = src;
  image.alt = name;
  body.append(image);
  PM.modal({
    title: name,
    body,
    width: Math.min(900, Math.max(320, window.innerWidth - 48)),
    actions: [{ label: 'Close' }]
  });
}

function attachmentBytes(item: Partial<PromptAttachment>): Uint8Array | null {
  const encoded = typeof item.dataBase64 === 'string'
    ? item.dataBase64
    : (typeof item.dataUrl === 'string' && item.dataUrl.includes(',') ? item.dataUrl.slice(item.dataUrl.indexOf(',') + 1) : null);
  if (encoded !== null) {
    try { return Uint8Array.from(window.atob(encoded), character => character.charCodeAt(0)); }
    catch { return null; }
  }
  if (typeof item.content === 'string') return new TextEncoder().encode(item.content);
  return null;
}

export async function activatePromptAttachment(PM: Record<string, any>, item: Record<string, any>): Promise<void> {
  if (item.dataUrl && isPreviewableImageType(item.type || item.dataUrl.slice(5, item.dataUrl.indexOf(';')))) {
    openImagePreview(PM, item.name, item.dataUrl);
    return;
  }
  try {
    if (item.sourcePath) {
      await bridge()!.media.revealSource(item.sourcePath);
      return;
    }
    const data = attachmentBytes(item);
    if (data) {
      await bridge()!.attachments.reveal({ name: item.name, data });
      return;
    }
    PM.toast(`The original location for ${item.name} is no longer available.`);
  } catch (error) {
    PM.toast(error instanceof Error ? error.message : `Could not reveal ${item.name} in Finder`, 6000);
  }
}

export function requestFileAttachments(items: PromptAttachment[]) {
  return items.filter(item => typeof item.dataBase64 === 'string' || typeof item.content === 'string')
    .map(({ name, type, dataBase64, content }) => ({ name, type, dataBase64, content }));
}

/** Small DOM adapter shared by the spatial composer and in-place renderer updates. */
export function mountPromptAttachments(PM: Record<string, any>, card: HTMLElement, items: () => PromptAttachment[]) {
  const row = card.querySelector('.spatial-input-row')!;
  const textarea = card.querySelector('textarea')!;
  const rail = document.createElement('div');
  rail.className = 'spatial-attachment-rail';
  rail.setAttribute('aria-label', 'Attachments');
  card.insertBefore(rail, row);
  const input = document.createElement('input');
  input.type = 'file'; input.multiple = true; input.hidden = true;
  input.setAttribute('aria-label', 'Attach files');
  const button = document.createElement('button');
  button.type = 'button'; button.className = 'spatial-attach'; button.title = ATTACHMENT_HINT;
  button.setAttribute('aria-label', 'Add attachments');
  button.append(PM.icon('plus'));
  row.prepend(input, button);
  button.onclick = () => input.click();

  function refresh() {
    rail.replaceChildren();
    for (const item of items()) {
      const chip = document.createElement('span'); chip.className = 'agent-attachment';
      const open = document.createElement('button'); open.type = 'button'; open.className = 'agent-attachment-open';
      open.setAttribute('aria-label', item.dataUrl ? `View ${item.name}` : `Reveal ${item.name} in Finder`);
      open.title = open.getAttribute('aria-label') || '';
      open.onclick = () => { void activatePromptAttachment(PM, item); };
      if (item.dataUrl) { const img = document.createElement('img'); img.src = item.dataUrl; img.alt = item.name; open.append(img); }
      else { const type = document.createElement('span'); type.className = 'agent-attachment-type'; type.textContent = (item.name.split('.').pop() || 'file').slice(0, 5).toUpperCase(); open.append(type); }
      const name = document.createElement('span'); name.className = 'agent-attachment-name'; name.textContent = item.name; name.title = item.name;
      open.append(name);
      const remove = document.createElement('button'); remove.type = 'button'; remove.className = 'agent-attachment-remove'; remove.setAttribute('aria-label', `Remove ${item.name}`); remove.append(PM.icon('x'));
      remove.onclick = () => { PM.AgentUI.removeAttachment(item.id); refresh(); textarea.focus(); };
      chip.append(open, remove); rail.append(chip);
    }
    rail.hidden = !items().length;
    card.classList.toggle('has-attachments', !!items().length);
  }
  let pendingReads = 0;
  let disposed = false;
  let previousReadOnly = false;
  let previousSendDisabled = false;
  const send = card.querySelector<HTMLButtonElement>('.spatial-send');
  async function add(files: File[]) {
    if (!files.length || disposed) return;
    if (pendingReads++ === 0) {
      previousReadOnly = textarea.readOnly;
      previousSendDisabled = send?.disabled ?? false;
    }
    button.disabled = true;
    if (send) send.disabled = true;
    textarea.readOnly = true;
    try { await PM.AgentUI.addAttachments(files); }
    catch (error) { PM.toast(String(error instanceof Error ? error.message : error), 6000); }
    finally {
      if (--pendingReads === 0) {
        button.disabled = false;
        if (send) send.disabled = previousSendDisabled;
        textarea.readOnly = previousReadOnly;
      }
      if (!disposed) {
        refresh();
        if (!pendingReads && card.isConnected) textarea.focus();
      }
    }
  }
  input.onchange = () => { void add(Array.from(input.files || [])); input.value = ''; };
  const paste = (event: ClipboardEvent) => {
    const files = Array.from(event.clipboardData?.files || []);
    if (files.length) { event.preventDefault(); void add(files); }
  };
  const dragover = (event: DragEvent) => { if (event.dataTransfer?.types.includes('Files')) event.preventDefault(); };
  const drop = (event: DragEvent) => { event.preventDefault(); event.stopPropagation(); void add(Array.from(event.dataTransfer?.files || [])); };
  const keydown = (event: KeyboardEvent) => { if (textarea.readOnly && event.key === 'Enter') { event.preventDefault(); event.stopImmediatePropagation(); } };
  textarea.addEventListener('paste', paste);
  textarea.addEventListener('keydown', keydown, true);
  card.addEventListener('dragover', dragover); card.addEventListener('drop', drop);
  refresh();
  return { refresh, dispose() {
    disposed = true;
    textarea.removeEventListener('paste', paste); textarea.removeEventListener('keydown', keydown, true);
    card.removeEventListener('dragover', dragover); card.removeEventListener('drop', drop);
    input.remove(); button.remove(); rail.remove();
  } };
}
