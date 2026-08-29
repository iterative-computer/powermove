export const ATTACHMENT_HINT = 'Attach images or files';

export interface PromptAttachment {
  id: string;
  name: string;
  type: string;
  size: number;
  dataUrl?: string;
  dataBase64?: string;
  content?: string;
}

export async function readPromptAttachment(file: File, id: string): Promise<PromptAttachment> {
  const imageTypes: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif' };
  const type = file.type || imageTypes[file.name.split('.').pop()?.toLowerCase() || ''] || 'application/octet-stream';
  const image = /^image\/(png|jpeg|webp|gif)$/.test(type);
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error(`Could not read ${file.name}.`));
    reader.onabort = () => reject(new Error(`Reading ${file.name} was cancelled.`));
    reader.readAsDataURL(file);
  });
  const item: PromptAttachment = { id, name: file.name || 'Attachment', type, size: file.size };
  if (image) item.dataUrl = dataUrl.replace(/^data:[^;]*;/, `data:${type};`);
  else {
    // Preserve binary bytes, including zero-byte files. Never attach just a filename.
    item.dataBase64 = dataUrl.slice(dataUrl.indexOf(',') + 1);
    if (type.startsWith('text/') || /\.(txt|md|json|js|ts|tsx|jsx|css|html?|svg|xml|csv|log)$/i.test(file.name)) {
      item.content = await file.text();
    }
  }
  return item;
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
      if (item.dataUrl) { const img = document.createElement('img'); img.src = item.dataUrl; img.alt = ''; chip.append(img); }
      const name = document.createElement('span'); name.className = 'agent-attachment-name'; name.textContent = item.name; name.title = item.name;
      const remove = document.createElement('button'); remove.type = 'button'; remove.setAttribute('aria-label', `Remove ${item.name}`); remove.append(PM.icon('x'));
      remove.onclick = () => { PM.AgentUI.removeAttachment(item.id); refresh(); textarea.focus(); };
      chip.append(name, remove); rail.append(chip);
    }
    rail.hidden = !items().length;
    card.classList.toggle('has-attachments', !!items().length);
  }
  async function add(files: File[]) {
    if (!files.length) return;
    button.disabled = true;
    const send = card.querySelector<HTMLButtonElement>('.spatial-send');
    if (send) send.disabled = true;
    textarea.readOnly = true;
    try { await PM.AgentUI.addAttachments(files); }
    catch (error) { PM.toast(String(error instanceof Error ? error.message : error), 6000); }
    finally { button.disabled = false; if (send) send.disabled = false; textarea.readOnly = false; refresh(); if (card.isConnected) textarea.focus(); }
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
    textarea.removeEventListener('paste', paste); textarea.removeEventListener('keydown', keydown, true);
    card.removeEventListener('dragover', dragover); card.removeEventListener('drop', drop);
    input.remove(); button.remove(); rail.remove();
  } };
}
