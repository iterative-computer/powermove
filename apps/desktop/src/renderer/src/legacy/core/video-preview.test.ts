import { expect, it, vi } from 'vitest';
import { prepareVideoPreview, previewVideoElement } from './video-preview';

it('uses editing media for Auto and reduced previews, and the original for Full and export', () => {
  const original = {}, preview = {}, asset = { el: original, preview: { el: preview } };
  const PM: any = { quality: 1, perf: { auto: true } };
  expect(previewVideoElement(PM, asset)).toBe(preview);
  PM.perf.auto = false;
  expect(previewVideoElement(PM, asset)).toBe(original);
  PM.quality = .5;
  expect(previewVideoElement(PM, asset)).toBe(preview);
  for (const state of [{ Export: { busy: true } }, { agentFrameCapture: true }, { Preview: { preparing: true } }]) {
    expect(previewVideoElement({ ...PM, ...state }, asset)).toBe(original);
  }
  expect(previewVideoElement(PM, { el: original })).toBe(original);
});


it('starts previews above 1 GB and releases the upload when the asset is disposed', async () => {
  let disposed = false;
  const media = {
    beginPreview: vi.fn(async () => { disposed = true; return 'preview'; }),
    releasePlaybackProxy: vi.fn(async () => {}),
  };
  vi.stubGlobal('window', { powermove: { media } });
  try {
    const size = 1024 * 1024 * 1024 + 1;
    await prepareVideoPreview({}, { w: 3840, h: 2160 }, { size } as Blob, () => disposed);
    expect(media.beginPreview).toHaveBeenCalledWith(size);
    expect(media.releasePlaybackProxy).toHaveBeenCalledWith('preview');
  } finally { vi.unstubAllGlobals(); }
});
