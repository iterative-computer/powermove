import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({ BrowserWindow: {}, clipboard: {} }));
import { IPC } from '../shared/ipc';
import { registerMediaPasteIpc } from './media-paste';

function harness(formats = ['image/png']) {
  let handle: (event: any) => Promise<boolean>;
  const types = vi.fn(async () => formats);
  const window = { isFocused: vi.fn(() => true), isDestroyed: vi.fn(() => false) };
  const trusted = vi.fn(() => true), acted = vi.fn(async () => true);
  registerMediaPasteIpc({ handle: vi.fn((channel, handler) => {
    expect(channel).toBe(IPC.mediaPaste); handle = handler;
  }) } as never, { isTrustedSender: trusted, windowFor: () => window, userActed: acted, types });
  const sender = { paste: vi.fn() };
  return { run: () => handle({ sender }), sender, types, window, trusted, acted };
}

describe('native media paste', () => {
  it.each(['image/png', 'video/mp4', 'audio/wav', 'text/uri-list', 'electron application/osclipboard;format="public.file-url"'])('requests native paste for %s without returning any clipboard data', async type => {
    const h = harness([type]);
    expect(await h.run()).toBe(true);
    expect(h.sender.paste).toHaveBeenCalledOnce();
  });
  it.each([{ formats: [] }, { formats: ['text/plain'] }, { formats: ['text/html'] }])('leaves the app clipboard available for non-media formats $formats', async ({ formats }) => {
    const h = harness(formats);
    expect(await h.run()).toBe(false);
    expect(h.sender.paste).not.toHaveBeenCalled();
  });
  it('rejects untrusted frames before accessing clipboard metadata', async () => {
    const h = harness(); h.trusted.mockReturnValue(false);
    await expect(h.run()).rejects.toThrow('Unauthorized');
    expect(h.types).not.toHaveBeenCalled();
  });
  it('requires focus and a recent user action', async () => {
    const h = harness(); h.window.isFocused.mockReturnValue(false);
    expect(await h.run()).toBe(false);
    h.window.isFocused.mockReturnValue(true); h.acted.mockResolvedValue(false);
    expect(await h.run()).toBe(false);
    expect(h.types).not.toHaveBeenCalled();
  });
  it('does not paste if the window loses focus while checking formats', async () => {
    const h = harness();
    h.types.mockImplementation(async () => { h.window.isFocused.mockReturnValue(false); return ['image/png']; });
    expect(await h.run()).toBe(false);
    expect(h.sender.paste).not.toHaveBeenCalled();
  });
});
