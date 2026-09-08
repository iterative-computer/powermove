// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { activatePromptAttachment, readPromptAttachment, requestFileAttachments } from './attachments';

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
