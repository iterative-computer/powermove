import { afterEach, expect, it, vi } from 'vitest';
import { disposeVideoInstances, layerVideoElement, pauseVideoInstances, pruneVideoInstances, videoInstanceTextureKey } from './video-instances';
import { seekPreviewVideo } from './video-seek';

function video(): HTMLVideoElement {
  return Object.assign(new EventTarget(), {
    src: 'blob:source', currentTime: 0, paused: true, seeking: false,
    pause: vi.fn(), load: vi.fn(), removeAttribute: vi.fn(),
  }) as unknown as HTMLVideoElement;
}
afterEach(() => vi.unstubAllGlobals());

it('keeps overlapping copies at distinct paused source times across repeated redraws', () => {
  const source = video(), copy = video();
  vi.stubGlobal('document', { createElement: vi.fn(() => copy) });
  const PM = { invalidate: vi.fn() }, asset = { el: source };
  for (let frame = 0; frame < 10; frame++) {
    seekPreviewVideo(layerVideoElement(PM, asset, 'first'), .25, .0005);
    seekPreviewVideo(layerVideoElement(PM, asset, 'second'), 1.25, .0005);
    expect(source.currentTime).toBe(.25);
    expect(copy.currentTime).toBe(1.25);
  }
  expect(document.createElement).toHaveBeenCalledTimes(1);
  expect(videoInstanceTextureKey(source)).not.toBe(videoInstanceTextureKey(copy));
  copy.dispatchEvent(new Event('loadeddata'));
  expect(PM.invalidate).toHaveBeenCalledWith('render');
  pauseVideoInstances(asset);
  expect(source.pause).toHaveBeenCalled(); expect(copy.pause).toHaveBeenCalled();
});

it('releases deleted clips and quality replacements without revoking shared media', () => {
  vi.stubGlobal('document', { createElement: vi.fn(video) });
  const PM = { perf: { auto: true }, invalidate: vi.fn() };
  const asset: any = { el: video() };
  const original = layerVideoElement(PM, asset, 'first');
  const copy = layerVideoElement(PM, asset, 'second');
  pruneVideoInstances(asset, new Set(['first']));
  expect(copy.removeAttribute).toHaveBeenCalledWith('src');
  expect(original.removeAttribute).not.toHaveBeenCalled();
  const replacement = layerVideoElement(PM, asset, 'second');
  expect(videoInstanceTextureKey(replacement)).not.toBe(videoInstanceTextureKey(copy));
  asset.preview = { el: video() };
  expect(layerVideoElement(PM, asset, 'first')).toBe(asset.preview.el);
  expect(original.pause).toHaveBeenCalled();
  expect(replacement.removeAttribute).toHaveBeenCalledWith('src');
  const previewCopy = layerVideoElement(PM, asset, 'second');
  disposeVideoInstances(asset);
  expect(previewCopy.removeAttribute).toHaveBeenCalledWith('src');
  expect(asset.preview.el.removeAttribute).not.toHaveBeenCalled();
  PM.invalidate.mockClear();
  previewCopy.dispatchEvent(new Event('loadeddata'));
  expect(PM.invalidate).not.toHaveBeenCalled();
});

it('releases inactive nested decoders while preserving another instance of the same layer', () => {
  vi.stubGlobal('document', { createElement: vi.fn(video) });
  const PM = { invalidate: vi.fn() }, asset = { el: video() };
  const first = layerVideoElement(PM, asset, 'first/clip');
  const second = layerVideoElement(PM, asset, 'second/clip');
  pruneVideoInstances(asset, new Set(['first/clip']), true);
  expect(second.removeAttribute).toHaveBeenCalledWith('src');
  expect(first.pause).not.toHaveBeenCalled();
  expect(layerVideoElement(PM, asset, 'first/clip')).toBe(first);
});
