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
