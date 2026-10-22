// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { previewRenderSize } from '../../../extensions/viewer/preview-size';
import { engineViewerCanvas, fillFrozenViewerCard, hideViewerPresentation, paintViewerCard, viewerCardFill, viewerCardSize, viewerCardTarget } from './viewer-preview';

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

/* A live viewer at 200% zoom, panned: #stage-inner is the full composition
   at that zoom, offset off-stage; #gl is a CSS crop of the visible part;
   #overlay is a stage-sized sibling of #stage-inner. */
const INNER_STYLE = 'width: 1280px; height: 720px; left: -213.5px; top: -97.25px;';
const GL_STYLE = 'position: absolute; left: 192px; top: 80px; width: 704px; height: 528px;';
function card(): { source: HTMLElement; clone: HTMLElement; inner: HTMLElement; gl: HTMLCanvasElement; overlay: HTMLCanvasElement } {
  const source = document.createElement('div');
  const clone = document.createElement('div');
  clone.innerHTML = `<style>.custom-overlay{color:red}</style><div data-library-source-id="stage"><div data-library-source-id="stage-inner" style="${INNER_STYLE}"><canvas data-library-source-id="gl" style="${GL_STYLE}"></canvas></div><canvas data-library-source-id="overlay" width="1280" height="960" style="width: 640px; height: 480px;"></canvas><span class="custom-overlay">Guide</span></div>`;
  document.body.append(source, clone);
  return {
    source, clone,
    inner: clone.querySelector('[data-library-source-id="stage-inner"]')!,
    gl: clone.querySelector('[data-library-source-id="gl"]')!,
    overlay: clone.querySelector('[data-library-source-id="overlay"]')!,
  };
}

function frameCanvas(width: number, height: number): HTMLCanvasElement {
  const frame = document.createElement('canvas'); frame.width = width; frame.height = height;
  return frame;
}

describe('viewer Library card', () => {
  it('covers the live stage-inner with the full composition and leaves its zoom, pan and overlay as cloned', () => {
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
    const { clone, inner, gl, overlay } = card();
    const overlayBefore = overlay.outerHTML;
    const glBefore = gl.getAttribute('style');
    const frame = frameCanvas(2560, 1440);

    expect(paintViewerCard(clone, frame)).toBe(true);

    const cover = inner.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-engine-frame]')!;
    expect(cover).not.toBeNull();
    expect(cover.previousElementSibling).toBe(gl);
    expect([cover.style.position, cover.style.left, cover.style.top, cover.style.width, cover.style.height])
      .toEqual(['absolute', '0px', '0px', '100%', '100%']);
    expect([cover.width, cover.height]).toEqual([2560, 1440]);
    expect(drawImage).toHaveBeenCalledWith(frame, 0, 0);
    // No refit: stage-inner keeps the live geometry, #gl its crop, the overlay its stage box.
    expect(inner.getAttribute('style')).toBe(INNER_STYLE);
    expect(gl.getAttribute('style')).toBe(glBefore);
    expect(overlay.outerHTML).toBe(overlayBefore);
    expect(overlay.parentElement).toBe(inner.parentElement);
    expect(clone.querySelector('.custom-overlay')?.textContent).toBe('Guide');
    expect(clone.querySelector('style')?.textContent).toContain('.custom-overlay');

    // A repaint reuses the cover.
    expect(paintViewerCard(clone, frameCanvas(1280, 720))).toBe(true);
    expect(inner.querySelectorAll('[data-library-engine-frame]')).toHaveLength(1);
    expect(inner.getAttribute('style')).toBe(INNER_STYLE);
  });

  describe('render size', () => {
    const project = { w: 1920, h: 1080 };
    const at = (zoom: number) => {
      const inner = document.createElement('div');
      inner.style.width = `${project.w * zoom}px`; inner.style.height = `${project.h * zoom}px`;
      return inner;
    };

    it.each([
      ['retina 100%', 1, 2, 1, 'auto', { width: 1920, height: 1080 }],
      ['retina fit 25%', 0.25, 2, 1, 'auto', { width: 960, height: 540 }],
      ['fit 25%', 0.25, 1, 1, 'auto', { width: 480, height: 270 }],
      ['400% at dpr 2', 4, 2, 1, 'auto', { width: 1920, height: 1080 }],
      ['800% at dpr 2', 8, 2, 1, 'auto', { width: 1920, height: 1080 }],
      ['dpr above 2', 0.25, 3, 1, 'auto', { width: 960, height: 540 }],
      ['retina half', 1, 2, 0.5, 'auto', { width: 960, height: 540 }],
      ['retina quarter', 1, 2, 0.25, 'auto', { width: 480, height: 270 }],
      ['quarter at fit 50%', 0.5, 1, 0.25, 'auto', { width: 240, height: 135 }],
      ['no fixed resolution', 0.5, 1, 1, null, { width: 960, height: 540 }],
      ['fixed full at fit 25%', 0.25, 1, 1, '1', { width: 1920, height: 1080 }],
      ['fixed half at 400%', 4, 2, 0.5, '0.5', { width: 960, height: 540 }],
      ['fixed quarter', 1, 2, 0.25, 0.25, { width: 480, height: 270 }],
    ] as const)('matches the live viewer render size: %s', (_, zoom, dpr, quality, resolution, expected) => {
      expect(viewerCardSize(at(zoom), project, dpr, quality, resolution)).toEqual(expected);
    });

    it('is the live previewRenderSize at the zoom the clone was taken at, never the zoomed CSS box', () => {
      for (const zoom of [0.02, 0.1, 0.25, 1 / 3, 0.5, 2, 8]) {
        for (const dpr of [1, 1.5, 2]) {
          for (const quality of [0.25, 0.5, 1]) {
            const live = previewRenderSize(project.w, project.h, zoom, dpr, quality);
            const size = viewerCardSize(at(zoom), project, dpr, quality, 'auto')!;
            expect(size).toEqual({ width: live.width, height: live.height });
            expect(size.width).toBeLessThanOrEqual(project.w);
            expect(size.height).toBeLessThanOrEqual(project.h);
          }
        }
      }
    });

    it('needs a stage-inner box and a composition', () => {
      expect(viewerCardSize(document.createElement('div'), project, 1, 1, 'auto')).toBeNull();
      expect(viewerCardSize(at(1), { w: 0, h: 1080 }, 1, 1, 'auto')).toBeNull();
    });
  });

  it('skips and hides the engine canvases a filled card does not copy', () => {
    expect(['gl', 'engine-present', 'overlay', ''].map(engineViewerCanvas)).toEqual([true, true, false, false]);
    const clone = document.createElement('div');
    clone.innerHTML = '<div data-library-source-id="stage"><div data-library-source-id="stage-inner"><canvas data-library-source-id="gl"></canvas></div><canvas data-library-source-id="engine-present" style="position: absolute; inset: 0; visibility: visible"></canvas><canvas data-library-source-id="overlay"></canvas></div>';
    hideViewerPresentation(clone);
    const visibility = (id: string) => clone.querySelector<HTMLCanvasElement>(`[data-library-source-id="${id}"]`)!.style.visibility;
    expect(visibility('engine-present')).toBe('hidden');
    expect(visibility('gl')).toBe('');
    expect(visibility('overlay')).toBe('');
  });

  it('targets only the known #gl inside #stage-inner, never another canvas', () => {
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext');
    const clone = document.createElement('div');
    clone.innerHTML = '<div data-library-source-id="stage"><canvas class="ruler"></canvas><div data-library-source-id="stage-inner"><canvas class="custom"></canvas></div><canvas data-library-source-id="gl"></canvas></div>';
    document.body.append(clone);
    expect(viewerCardTarget(clone)).toBeNull();
    expect(paintViewerCard(clone, frameCanvas(10, 10))).toBe(false);
    expect(clone.querySelector('[data-library-engine-frame]')).toBeNull();
    expect(getContext).not.toHaveBeenCalled();
  });

  it('leaves a viewer with its own Library render alone and fills only one without a callback', () => {
    const enginePreview = Object.assign(() => {}, { enginePreview: true });
    expect(viewerCardFill('viewer', true, undefined)).toEqual({ skipGl: true, host: true });
    expect(viewerCardFill('viewer', true, { width: 640 } as never)).toEqual({ skipGl: true, host: true });
    expect(viewerCardFill('viewer', true, { render: enginePreview })).toEqual({ skipGl: true, host: false });
    // A customised fork that draws its own card keeps base behaviour.
    expect(viewerCardFill('viewer', true, { render: () => {} })).toEqual({ skipGl: false, host: false });
    expect(viewerCardFill('viewer', false, undefined)).toEqual({ skipGl: false, host: false });
    expect(viewerCardFill('timeline', true, undefined)).toEqual({ skipGl: false, host: false });
  });

  describe('frozen fork fill', () => {
    function harness() {
      const drawImage = vi.fn();
      vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage } as never);
      const parts = card();
      const project = { id: 'project', compId: 'main', w: 640, h: 360 };
      const finishers: Array<(value: unknown) => void> = [];
      const frame = frameCanvas(1280, 720);
      const state = { revision: 7 };
      const engine = {
        currentRevision: () => state.revision,
        thumbnails: { request: vi.fn(() => new Promise((resolve) => { finishers.push(resolve); })) },
        toCanvas: vi.fn(() => frame),
      };
      const PM: Record<string, any> = { EngineRender: engine, proj: project, time: 0.5, quality: 1, previewResolution: 'auto', panelInst: { viewer: { el: parts.source } } };
      return { ...parts, drawImage, project, finishers, frame, state, engine, PM };
    }
    const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

    it('requests the full composition at the live render size with the paused viewer motion blur and no draft 3D', async () => {
      vi.stubGlobal('devicePixelRatio', 1);
      const h = harness();
      const before = h.inner.getAttribute('style');
      fillFrozenViewerCard(h.PM, h.source, h.clone);
      expect(h.engine.thumbnails.request).toHaveBeenCalledTimes(1);
      const request = (h.engine.thumbnails.request.mock.calls[0] as unknown[])[0] as Record<string, unknown>;
      // A 640×360 composition at 200%: the composition's own size, not the 1280×720 CSS box.
      expect(request).toMatchObject({ key: 'library-viewer:project', time: 0.5, width: 640, height: 360, motionBlurSamples: 12, minRevision: 7 });
      expect(request).not.toHaveProperty('draft3d');
      expect(request).not.toHaveProperty('composition');
      h.finishers[0]!({ revision: 7 });
      await settle();
      expect(h.drawImage).toHaveBeenCalledWith(h.frame, 0, 0);
      expect(h.inner.getAttribute('style')).toBe(before);
      vi.unstubAllGlobals();
    });

    it.each([
      ['retina 200%', 2, 1, 'auto', [640, 360]],
      ['half quality', 1, 0.5, 'auto', [320, 180]],
      ['quarter quality', 2, 0.25, 'auto', [160, 90]],
      ['fixed half resolution', 2, 0.5, '0.5', [320, 180]],
    ] as const)('sizes a %s request from the host quality and resolution', (_, dpr, quality, resolution, [width, height]) => {
      vi.stubGlobal('devicePixelRatio', dpr);
      const h = harness();
      Object.assign(h.PM, { quality, previewResolution: resolution });
      fillFrozenViewerCard(h.PM, h.source, h.clone);
      expect(h.engine.thumbnails.request).toHaveBeenCalledWith(expect.objectContaining({ width, height }));
      vi.unstubAllGlobals();
    });

    it('finishes into a clone the Library detached while the request was pending, and its cached return asks nothing', async () => {
      const h = harness();
      fillFrozenViewerCard(h.PM, h.source, h.clone);
      expect(h.engine.thumbnails.request).toHaveBeenCalledTimes(1);
      // A shelf switch or search keystroke detaches the cached card.
      h.clone.remove();
      h.finishers[0]!({ revision: 7 });
      await settle();
      expect(h.drawImage).toHaveBeenCalledWith(h.frame, 0, 0);
      const cover = h.inner.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-engine-frame]')!;
      expect([cover.width, cover.height]).toEqual([1280, 720]);
      // Coming back re-attaches the cached clone as is: its picture is there and nothing renders again.
      document.body.append(h.clone);
      await new Promise((resolve) => setTimeout(resolve, 250));
      expect(h.engine.thumbnails.request).toHaveBeenCalledTimes(1);
      expect(h.inner.querySelectorAll('[data-library-engine-frame]')).toHaveLength(1);
    });

    it('rejects a result from another revision, time, composition or source panel', async () => {
      const h = harness();
      fillFrozenViewerCard(h.PM, h.source, h.clone);
      h.state.revision = 8;
      h.finishers[0]!({ revision: 7 });
      await settle();
      expect(h.drawImage).not.toHaveBeenCalled();

      fillFrozenViewerCard(h.PM, h.source, h.clone);
      h.PM.time = 1;
      h.finishers[1]!({ revision: 8 });
      await settle();
      expect(h.drawImage).not.toHaveBeenCalled();

      fillFrozenViewerCard(h.PM, h.source, h.clone);
      h.project.compId = 'other';
      h.finishers[2]!({ revision: 8 });
      await settle();
      expect(h.drawImage).not.toHaveBeenCalled();
      h.project.compId = 'main';

      fillFrozenViewerCard(h.PM, h.source, h.clone);
      h.PM.panelInst.viewer = { el: document.createElement('div') };
      h.finishers[3]!({ revision: 8 });
      await settle();
      expect(h.drawImage).not.toHaveBeenCalled();
      expect(h.clone.querySelector('[data-library-engine-frame]')).toBeNull();

      h.PM.panelInst.viewer = { el: h.source };
      fillFrozenViewerCard(h.PM, h.source, h.clone);
      h.finishers[4]!({ revision: 8 });
      await settle();
      expect(h.drawImage).toHaveBeenCalledWith(h.frame, 0, 0);
    });

    it('does nothing without the known #gl', () => {
      const h = harness();
      h.clone.querySelector('[data-library-source-id="gl"]')!.remove();
      fillFrozenViewerCard(h.PM, h.source, h.clone);
      expect(h.engine.thumbnails.request).not.toHaveBeenCalled();
    });
  });
});
