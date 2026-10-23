// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { copyClonedCanvasBitmaps, renderLibraryCallback } from './viewer-preview';

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

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
