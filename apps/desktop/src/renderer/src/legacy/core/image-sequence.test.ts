// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { convertImageSequence } from './image-sequence';

afterEach(() => { vi.unstubAllGlobals(); delete (window as any).powermove; });
const frames = () => [new File(['a'], 'f1.png'), new File(['b'], 'f2.png')];

it('rejects inconsistent dimensions and closes each decoded bitmap before native conversion', async () => {
  const close = vi.fn(), createImageSequence = vi.fn();
  (window as any).powermove = { media: { createImageSequence } };
  vi.stubGlobal('createImageBitmap', vi.fn().mockResolvedValueOnce({ width: 64, height: 64, close })
    .mockResolvedValueOnce({ width: 32, height: 64, close }));
  await expect(convertImageSequence(frames(), 30, () => {})).rejects.toThrow('Frame dimensions differ: f2.png');
  expect(close).toHaveBeenCalledTimes(2);
  expect(createImageSequence).not.toHaveBeenCalled();
});

it('releases a finished native conversion if the project changed while it was running', async () => {
  let changed = false;
  const releasePlaybackProxy = vi.fn(async () => {}), readPlaybackProxy = vi.fn();
  (window as any).powermove = { media: {
    createImageSequence: async () => { changed = true; return { ok: true, token: 'test-token', size: 9, type: 'video/webm' }; },
    releasePlaybackProxy, readPlaybackProxy,
  } };
  vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ width: 64, height: 64, close() {} })));
  await expect(convertImageSequence(frames(), 30, () => { if (changed) throw new Error('switched projects'); })).rejects.toThrow('switched projects');
  expect(readPlaybackProxy).not.toHaveBeenCalled();
  expect(releasePlaybackProxy).toHaveBeenCalledWith('test-token');
});
