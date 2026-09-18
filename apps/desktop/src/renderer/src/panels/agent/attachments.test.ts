// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { activatePromptAttachment, mountPromptAttachments, readPromptAttachment, requestFileAttachments } from './attachments';

describe('prompt attachments', () => {
  it('keeps arbitrary binary files byte for byte', async () => {
    const data = new Uint8Array([0, 255, 128, 10, 13, 80, 68, 70]);
    const item = await readPromptAttachment(new File([data], 'reference.pdf', { type: 'application/pdf' }), 'one');
    expect(item.dataUrl).toBeUndefined();
    expect([...Buffer.from(item.dataBase64!, 'base64')]).toEqual([...data]);
    expect(requestFileAttachments([item])[0]?.dataBase64).toBe(item.dataBase64);
  });
  it('keeps empty files and exact UTF-8 text', async () => {
    const empty = await readPromptAttachment(new File([], 'empty.bin'), 'empty');
    const text = await readPromptAttachment(new File(['Café 🎬'], 'brief.txt', { type: 'text/plain' }), 'text');
    expect(requestFileAttachments([empty, text])).toHaveLength(2);
    expect(text.content).toBe('Café 🎬');
    expect(Buffer.from(text.dataBase64!, 'base64').toString('utf8')).toBe(text.content);
  });
  it('routes images to visual references without arbitrary byte limits', async () => {
    const image = await readPromptAttachment(new File(['png'], 'reference.png', { type: 'image/png' }), 'image');
    expect(image.dataUrl).toMatch(/^data:image\/png;base64,/);
    expect(requestFileAttachments([image])).toEqual([]);
    const largeFile = await readPromptAttachment(new File([new Uint8Array(100 * 1024 + 1)], 'large.zip'), 'zip');
    const largeImage = await readPromptAttachment(new File([new Uint8Array(4 * 1024 * 1024 + 1)], 'large.png', { type: 'image/png' }), 'png');
    expect(largeFile.dataBase64).toBeTruthy();
    expect(largeImage.dataUrl).toMatch(/^data:image\/png;base64,/);
  });
  it('previews displayable image attachments and reveals source or cached file bytes', async () => {
    const modal = vi.fn();
    const toast = vi.fn();
    const revealSource = vi.fn();
    const reveal = vi.fn();
    Object.defineProperty(window, 'powermove', { configurable: true, value: { media: { revealSource }, attachments: { reveal } } });

    await activatePromptAttachment({ modal, toast }, { name: 'reference.svg', type: 'image/svg+xml', dataUrl: 'data:image/svg+xml;base64,PHN2Zy8+' });
    expect(modal).toHaveBeenCalledOnce();
    expect(modal.mock.calls[0]?.[0].body.querySelector('img')?.alt).toBe('reference.svg');

    await activatePromptAttachment({ modal, toast }, { name: 'brief.pdf', sourcePath: '/Users/editor/brief.pdf' });
    expect(revealSource).toHaveBeenCalledWith('/Users/editor/brief.pdf');
    await activatePromptAttachment({ modal, toast }, { name: 'pasted.bin', dataBase64: 'AP8=' });
    expect(reveal).toHaveBeenCalledWith({ name: 'pasted.bin', data: new Uint8Array([0, 255]) });
    await activatePromptAttachment({ modal, toast }, { name: 'old.txt' });
    expect(toast).toHaveBeenCalledWith('The original location for old.txt is no longer available.');
  });
});

it.each(['constructor', '__proto__'])('reads unknown file extension %s as binary', async extension => {
  const item = await readPromptAttachment(new File(['data'], `reference.${extension}`), 'one');
  expect(item.type).toBe('application/octet-stream');
  expect(Buffer.from(item.dataBase64!, 'base64').toString()).toBe('data');
});

it.each([false, true])('keeps the composer locked until overlapping attachment reads finish (initially locked: %s)', async initiallyLocked => {
  const pending: Array<() => void> = [];
  const card = document.createElement('div');
  card.innerHTML = '<div class="spatial-input-row"><textarea></textarea><button class="spatial-send"></button></div>';
  const PM = { icon: () => document.createElement('span'), toast: vi.fn(), AgentUI: {
    addAttachments: vi.fn(() => new Promise<void>(resolve => pending.push(resolve)))
  } };
  const textarea = card.querySelector('textarea')!;
  textarea.readOnly = initiallyLocked;
  card.querySelector<HTMLButtonElement>('.spatial-send')!.disabled = initiallyLocked;
  const mounted = mountPromptAttachments(PM, card, () => []);
  for (let i = 0; i < 2; i++) {
    const event = new Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: { files: [new File(['x'], 'a.txt')] } });
    textarea.dispatchEvent(event);
  }
  expect(PM.AgentUI.addAttachments).toHaveBeenCalledTimes(2);
  pending[0]!();
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(textarea.readOnly).toBe(true);
  expect(card.querySelector<HTMLButtonElement>('.spatial-send')!.disabled).toBe(true);
  pending[1]!();
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(textarea.readOnly).toBe(initiallyLocked);
  expect(card.querySelector<HTMLButtonElement>('.spatial-send')!.disabled).toBe(initiallyLocked);
  mounted.dispose();
});

it('restores composer controls and reports a failed attachment read', async () => {
  const card = document.createElement('div');
  card.innerHTML = '<div class="spatial-input-row"><textarea></textarea><button class="spatial-send"></button></div>';
  const PM = { icon: () => document.createElement('span'), toast: vi.fn(), AgentUI: {
    addAttachments: vi.fn().mockRejectedValue(new Error('File unavailable'))
  } };
  const mounted = mountPromptAttachments(PM, card, () => []);
  const event = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', { value: { files: [new File(['x'], 'a.txt')] } });
  card.querySelector('textarea')!.dispatchEvent(event);
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(PM.toast).toHaveBeenCalledWith('File unavailable', 6000);
  expect(card.querySelector('textarea')!.readOnly).toBe(false);
  expect(card.querySelector<HTMLButtonElement>('.spatial-send')!.disabled).toBe(false);
  mounted.dispose();
});
