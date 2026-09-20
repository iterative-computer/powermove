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

it('stops reading generated media immediately when its project is closed', async () => {
  let disposed = false;
  const media = {
    beginPreview: vi.fn(async () => 'preview'), writePreview: vi.fn(async () => {}),
    finishPreview: vi.fn(async () => ({ size: 2 * 1024 * 1024 })),
    readPlaybackProxy: vi.fn(async () => { disposed = true; return new Uint8Array(1024 * 1024); }),
    releasePlaybackProxy: vi.fn(async () => {}),
  };
  vi.stubGlobal('window', { powermove: { media } });
  const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
  try {
    await prepareVideoPreview({}, { w: 3840, h: 2160 }, new Blob(['source']), () => disposed);
    expect(media.readPlaybackProxy).toHaveBeenCalledTimes(1);
    expect(media.releasePlaybackProxy).toHaveBeenCalledWith('preview');
    expect(warn).not.toHaveBeenCalled();
  } finally { vi.unstubAllGlobals(); warn.mockRestore(); }
});

it('prepares an older HD alpha proxy without waiting behind optional 4K work', async () => {
  let finish!: (value: { size: number }) => void, entered!: () => void;
  const waiting = new Promise<void>(resolve => { entered = resolve; });
  const conversion = new Promise<{ size: number }>(resolve => { finish = resolve; });
  let firstDisposed = false, legacyDisposed = false;
  const media = {
    beginPreview: vi.fn(async () => {
      if (media.beginPreview.mock.calls.length === 1) return '4k';
      legacyDisposed = true; return 'legacy';
    }),
    writePreview: vi.fn(async () => {}),
    finishPreview: vi.fn(async () => { entered(); return conversion; }),
    releasePlaybackProxy: vi.fn(async () => {}),
  };
  vi.stubGlobal('window', { powermove: { media } });
  const first = prepareVideoPreview({}, { w: 3840, h: 2160 }, new Blob(['source']), () => firstDisposed);
  try {
    await waiting;
    await prepareVideoPreview({}, { w: 1920, h: 1080, playbackProxy: true, playbackProxyVersion: 2 }, new Blob(['cutout']), () => legacyDisposed);
    expect(media.beginPreview).toHaveBeenCalledTimes(2);
    expect(media.releasePlaybackProxy).toHaveBeenCalledWith('legacy');
  } finally {
    firstDisposed = true; finish({ size: 1 }); await first; vi.unstubAllGlobals();
  }
});

it('persists editing media separately and reuses it after reload without conversion', async () => {
  const store = new Map<string, Blob>();
  const media = {
    beginPreview: vi.fn(async () => 'preview'), writePreview: vi.fn(async () => {}),
    finishPreview: vi.fn(async () => ({ size: 3 })),
    readPlaybackProxy: vi.fn(async () => new Uint8Array([1, 2, 3])),
    releasePlaybackProxy: vi.fn(async () => {}),
  };
  const PM: any = {
    MediaStore: {
      get: vi.fn(async (key: string) => store.get(key)),
      put: vi.fn(async (key: string, blob: Blob) => { store.set(key, blob); return true; }),
    },
    bus: { emit: vi.fn() }, invalidate: vi.fn(),
  };
  const makeAsset = () => ({
    w: 1920, h: 1080, playbackProxy: true, playbackProxyVersion: 2,
    storageKey: 'media:original', el: { pause: vi.fn() },
  } as any);
  const source = new Blob(['original bytes']); store.set('media:original', source);
  vi.stubGlobal('window', { powermove: { media } });
  vi.stubGlobal('document', { createElement: () => {
    const el: any = new EventTarget();
    let frame: () => void;
    Object.defineProperty(el, 'src', { set: () => queueMicrotask(() => el.dispatchEvent(new Event('loadeddata'))) });
    el.requestVideoFrameCallback = (cb: () => void) => { frame = cb; };
    el.play = async () => { frame(); };
    el.pause = vi.fn();
    return el;
  } });
  try {
    const first = makeAsset(); await prepareVideoPreview(PM, first, source, () => false);
    expect(first.preview).toBeDefined();
    expect(media.beginPreview).toHaveBeenCalledOnce();
    const reloaded = makeAsset(); await prepareVideoPreview(PM, reloaded, source, () => false);
    expect(reloaded.preview).toBeDefined();
    expect(media.beginPreview).toHaveBeenCalledOnce();
    expect(store.get('media:original')).toBe(source);
    expect(PM.MediaStore.put).toHaveBeenCalledOnce();
    URL.revokeObjectURL(first.preview.url); URL.revokeObjectURL(reloaded.preview.url);
  } finally { vi.unstubAllGlobals(); }
});
