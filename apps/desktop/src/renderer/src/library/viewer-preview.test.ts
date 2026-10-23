// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { copyClonedCanvasBitmaps, snapshotClonedCanvasBitmaps, renderLibraryCallback } from './viewer-preview';
import { EngineSnapshotError, type EngineSnapshot } from '../engine-connection/render-transport';
import type { SnapshotViewer } from '../../../shared/engine-protocol';

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); vi.useRealTimers(); });

function pair(sourceMarkup: string): { source: HTMLElement; clone: HTMLElement } {
  const source = document.createElement('div');
  source.innerHTML = sourceMarkup;
  const clone = source.cloneNode(true) as HTMLElement;
  document.body.append(source, clone);
  return { source, clone };
}

describe('Library canvas copy', () => {
  it('copies the native stage bitmap at its source CSS size before clone layout can resize it', () => {
    const { source, clone } = pair('<div class="stage"><canvas id="engine-present" width="1496" height="904" style="position:absolute;inset:0;width:100%;height:100%"></canvas><canvas id="overlay" width="1496" height="904" style="width:748px;height:452px"></canvas></div>');
    const native = source.querySelector<HTMLCanvasElement>('#engine-present')!;
    const clonedNative = clone.querySelector<HTMLCanvasElement>('#engine-present')!;
    const overlay = clone.querySelector<HTMLCanvasElement>('#overlay')!;
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
    vi.spyOn(window, 'getComputedStyle').mockImplementation((element) => ({
      width: element === native ? '748px' : '748px', height: '452px',
    }) as CSSStyleDeclaration);

    copyClonedCanvasBitmaps(source, clone);

    expect([clonedNative.width, clonedNative.height]).toEqual([1496, 904]);
    expect([clonedNative.style.width, clonedNative.style.height]).toEqual(['748px', '452px']);
    expect(drawImage).toHaveBeenCalledWith(native, 0, 0);
    expect([overlay.width, overlay.height, overlay.style.width, overlay.style.height]).toEqual([1496, 904, '748px', '452px']);
  });

  it('copies a stage-level presentation as today\'s composition canvas, at its box and backing size', () => {
    const { source, clone } = pair('<div id="stage"><div id="stage-inner"></div><canvas id="engine-present" data-todays-canvas="24 62 640 360 1280 720" width="1496" height="904" style="position:absolute;inset:0;width:100%;height:100%"></canvas></div>');
    const native = source.querySelector<HTMLCanvasElement>('#engine-present')!;
    const copy = clone.querySelector<HTMLCanvasElement>('canvas')!;
    const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} }) as DOMRect;
    vi.spyOn(native, 'getBoundingClientRect').mockReturnValue(rect(100, 50, 748, 452));
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
    const computed = vi.spyOn(window, 'getComputedStyle');

    copyClonedCanvasBitmaps(source, clone);

    expect([copy.width, copy.height]).toEqual([1280, 720]);
    expect([copy.style.left, copy.style.top, copy.style.width, copy.style.height]).toEqual(['24px', '62px', '640px', '360px']);
    // The composition's pixels on the presentation: 2 backing px per CSS px.
    expect(drawImage).toHaveBeenCalledWith(native, 48, 124, 1280, 720, 0, 0, 1280, 720);
    expect(computed).not.toHaveBeenCalled();
  });

  it('keeps a presentation without today\'s canvas box (bound inside a viewer\'s own stage-inner) at its own box', () => {
    const { source, clone } = pair('<div id="stage-inner"><canvas id="gl" width="1280" height="720"></canvas><canvas id="engine-present" width="1280" height="720" style="position:absolute;inset:0;width:100%;height:100%"></canvas></div>');
    const native = source.querySelector<HTMLCanvasElement>('#engine-present')!;
    const copy = clone.querySelector<HTMLCanvasElement>('#engine-present')!;
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
    vi.spyOn(window, 'getComputedStyle').mockReturnValue({ width: '640px', height: '360px' } as CSSStyleDeclaration);

    copyClonedCanvasBitmaps(source, clone);

    expect([copy.width, copy.height, copy.style.width, copy.style.height]).toEqual([1280, 720, '640px', '360px']);
    expect(drawImage).toHaveBeenCalledWith(native, 0, 0);
  });

  it('keeps ordinary canvas pixels and lets a custom library.render own its card afterward', () => {
    const { source, clone } = pair('<canvas width="320" height="180" style="width:100%;height:50%"></canvas>');
    const original = source.querySelector('canvas')!;
    const copy = clone.querySelector('canvas')!;
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
    const computed = vi.spyOn(window, 'getComputedStyle').mockReturnValue({ width: '320px', height: '180px' } as CSSStyleDeclaration);
    const customRender = vi.fn(({ clone: card }: { clone: HTMLElement }) => { card.querySelector('canvas')!.style.width = '160px'; });

    copyClonedCanvasBitmaps(source, clone);
    expect(renderLibraryCallback({ render: customRender }, { source, clone, width: 640, height: 480 })).toBe(true);
    expect(renderLibraryCallback(undefined, { source, clone, width: 640, height: 480 })).toBe(false);

    expect(drawImage).toHaveBeenCalledWith(original, 0, 0);
    expect(computed).not.toHaveBeenCalled();
    expect(customRender).toHaveBeenCalledOnce();
    expect([copy.width, copy.height, copy.style.width, copy.style.height]).toEqual([320, 180, '160px', '50%']);
  });
});

describe('Library engine snapshots', () => {
  function capture() {
    const { source, clone } = pair('<canvas id="engine-present" width="320" height="180"></canvas><canvas id="overlay" width="640" height="360"></canvas>');
    const bitmap = document.createElement('canvas');
    const result = { width: 320, height: 180, revision: 12 } as EngineSnapshot;
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
    const transport = {
      thumbnails: { request: vi.fn().mockResolvedValue(result) },
      currentRevision: vi.fn(() => 12), toCanvas: vi.fn(() => bitmap),
    };
    const current = vi.fn(() => true);
    return { source, clone, bitmap, drawImage, transport, current, result };
  }

  it('renders a pinned cropped editor canvas without reusing the stage scaling or a newer view', async () => {
    const f = capture();
    const viewer: SnapshotViewer = { view: { camera: null, shading: 'solid', xray: true, localView: ['cube'] }, viewSeq: 7,
      canvas: { width: 160, height: 90, crop: { x: 80, y: 40, width: 80, height: 45 }, css: { x: 20, y: 10, width: 320, height: 180 } } };
    const pin = structuredClone(viewer);
    Object.assign(f.result, { width: 160, height: 90, viewSeq: 7 });
    const pending = snapshotClonedCanvasBitmaps(f.source, f.clone, { ...f, transport: f.transport as never, composition: 'comp', time: .5, viewer });
    viewer.viewSeq = 8; viewer.canvas.crop.x = 0;
    expect(await pending).toBe(true);
    expect(f.transport.thumbnails.request).toHaveBeenCalledWith(expect.objectContaining({ width: 160, height: 90, viewer: pin }));
    expect(f.drawImage).toHaveBeenCalledExactlyOnceWith(f.bitmap, 0, 0);
    expect([f.clone.querySelector('canvas')!.width, f.clone.querySelector('canvas')!.height]).toEqual([160, 90]);
  });

  it('does not combine a pinned view’s chrome with pixels tagged as another view', async () => {
    const f = capture();
    const viewer: SnapshotViewer = { view: null, viewSeq: 7,
      canvas: { width: 320, height: 180, crop: { x: 0, y: 0, width: 320, height: 180 }, css: { x: 0, y: 0, width: 640, height: 360 } } };
    f.result.viewSeq = 8;
    expect(await snapshotClonedCanvasBitmaps(f.source, f.clone, { ...f, transport: f.transport as never, composition: 'comp', time: .5, viewer })).toBe(false);
    expect(f.drawImage).not.toHaveBeenCalled();
  });

  it('renders at the cloned backing size and keeps the live overlay untouched', async () => {
    const f = capture();
    expect(await snapshotClonedCanvasBitmaps(f.source, f.clone, {
      ...f, transport: f.transport as never, composition: 'comp', time: .5,
    })).toBe(true);
    expect(f.transport.thumbnails.request).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      key: expect.stringMatching(/^library-viewer:/), composition: 'comp', time: .5,
      width: 320, height: 180, minRevision: 12, pixels: 'opaque', draft3d: true,
    }));
    expect(f.drawImage).toHaveBeenCalledExactlyOnceWith(f.bitmap, 0, 0);
    expect(f.clone.querySelector('#overlay')!.getAttribute('width')).toBe('640');
  });

  it('uses the original WebGL backing size for a pre-engine fork, before stage presentation scales it', async () => {
    const f = capture();
    const original = document.createElement('canvas'); original.width = 160; original.height = 90;
    f.result.width = 160; f.result.height = 90;
    f.source.prepend(original);
    f.clone.prepend(original.cloneNode(true));
    expect(await snapshotClonedCanvasBitmaps(f.source, f.clone, {
      ...f, transport: f.transport as never, composition: 'comp', time: .5,
    })).toBe(true);
    expect(f.transport.thumbnails.request).toHaveBeenCalledWith(expect.objectContaining({ width: 160, height: 90 }));
    expect(f.clone.querySelector('#engine-present')).toBeNull();
    const picture = f.clone.querySelector<HTMLCanvasElement>('[data-library-engine-snapshot]')!;
    expect([picture.width, picture.height, picture.style.visibility]).toEqual([160, 90, 'visible']);
  });

  it('keeps the card capture pending through a transient session failure and paints only the recovered snapshot', async () => {
    vi.useFakeTimers();
    const f = capture();
    f.transport.thumbnails.request.mockRejectedValueOnce(new EngineSnapshotError('unavailable', 'Session restarted'));
    const pending = snapshotClonedCanvasBitmaps(f.source, f.clone, {
      ...f, transport: f.transport as never, composition: 'comp', time: .5,
    });
    await vi.advanceTimersByTimeAsync(249);
    expect(f.drawImage).not.toHaveBeenCalled();
    expect(f.transport.thumbnails.request).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(await pending).toBe(true);
    expect(f.transport.thumbnails.request).toHaveBeenCalledTimes(2);
    expect(f.drawImage).toHaveBeenCalledExactlyOnceWith(f.bitmap, 0, 0);
  });

  it('drops a capture that becomes stale while waiting to retry', async () => {
    vi.useFakeTimers();
    const f = capture();
    f.transport.thumbnails.request.mockRejectedValueOnce(new EngineSnapshotError('resyncPending', 'Resyncing'));
    const pending = snapshotClonedCanvasBitmaps(f.source, f.clone, {
      ...f, transport: f.transport as never, composition: 'comp', time: .5,
    });
    await vi.advanceTimersByTimeAsync(1);
    f.current.mockReturnValue(false);
    await vi.advanceTimersByTimeAsync(250);
    expect(await pending).toBe(false);
    expect(f.transport.thumbnails.request).toHaveBeenCalledTimes(1);
    expect(f.drawImage).not.toHaveBeenCalled();
  });

  it('surfaces persistent snapshot failures after three attempts without painting an inexact fallback', async () => {
    vi.useFakeTimers();
    const f = capture();
    const error = new EngineSnapshotError('timeout', 'Snapshot timed out');
    f.transport.thumbnails.request.mockRejectedValue(error);
    const pending = snapshotClonedCanvasBitmaps(f.source, f.clone, {
      ...f, transport: f.transport as never, composition: 'comp', time: .5,
    });
    const rejected = expect(pending).rejects.toBe(error);
    await vi.advanceTimersByTimeAsync(500);
    await rejected;
    expect(f.transport.thumbnails.request).toHaveBeenCalledTimes(3);
    expect(f.drawImage).not.toHaveBeenCalled();
  });

  it.each(['revision', 'closed', 'superseded'])('discards a %s capture before it can paint the clone', async reason => {
    const f = capture();
    if (reason === 'revision') f.transport.currentRevision.mockReturnValueOnce(12).mockReturnValue(13);
    if (reason === 'closed') f.current.mockReturnValue(false);
    if (reason === 'superseded') f.transport.thumbnails.request.mockResolvedValue(null);
    expect(await snapshotClonedCanvasBitmaps(f.source, f.clone, {
      ...f, transport: f.transport as never, composition: 'comp', time: .5,
    })).toBe(false);
    expect(f.drawImage).not.toHaveBeenCalled();
  });
});
