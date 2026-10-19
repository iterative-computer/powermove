/*
 * `render.gl.init(canvas)` and `render.gl.resize(…)` with "Use New Engine"
 * on (plan section 18 item 7, section 20 "render.gl.init, resize: bind
 * engine presentation to the passed canvas"; the Phase 3c render
 * transport).
 *
 * Today a viewer creates a canvas, hands it to `render.gl.init`, and sizes
 * it with `render.gl.resize`; the compositor draws into it. Old custom
 * viewers (forks of the built-in viewer frozen before the engine, and
 * overrides of it by id) still do exactly that. With the engine on, the
 * binding puts an engine presentation over the canvas the caller passed:
 * the composition drawn where that canvas sits in its parent, at the size
 * the caller gave it, under the caller's own overlay (the presentation's
 * canvas goes right after the passed canvas, so elements after it stay on
 * top), and the passed canvas rests hidden.
 *
 * No competing presenters: a viewer that presents by itself (the built-in
 * viewer marks its stage `data-engine-self-presents` while mounted, and its
 * presentation marks it `data-engine-stage="viewer"`) owns the canvas, and
 * the binding yields to it, at bind time and at every resize. A new `init` with another canvas replaces the binding (one per
 * window), and a canvas that left the document unbinds: at the next
 * `resize`, or as soon as its size observer sees it go (a caller that
 * tears its canvas down calls nothing more; its presentation would keep a
 * presenting engine session for a stage nobody sees).
 */
import type { EngineViewerService, ViewParams } from '../../../extensions/viewer/engine-present/presentation';

/** Mirrors engine-present/presentation.ts `ENGINE_STAGE_EVENT`. */
export const ENGINE_STAGE_EVENT = 'powermove:engine-stage';
/** Mirrors engine-present/presentation.ts `ENGINE_SELF_PRESENTS_ATTRIBUTE`. */
export const ENGINE_SELF_PRESENTS_ATTRIBUTE = 'data-engine-self-presents';

/** What the binding needs of a presentation (`ViewerPresentation`). */
export interface BoundPresentation {
  readonly exclusive: boolean;
  update(params: ViewParams, stage?: { width: number; height: number }): void;
  dispose(): void;
}

export type CanvasBindingOptions = {
  /** The kernel's `engine.viewer` service (null while the engine is off). */
  service: () => EngineViewerService | null;
  /** The open composition's size in composition pixels. */
  composition: () => { w: number; h: number } | null;
  /** The preview quality (today's `PM.quality`: 1, 0.5, 0.25). */
  quality: () => number;
  /** Makes the presentation (`new ViewerPresentation(…)`). */
  present: (options: { service: EngineViewerService; stage: HTMLElement; before: Element | null; role: 'binding' }) => BoundPresentation;
  /** Calls `changed` when `canvas` may have left the document; returns the
   *  stop. Default: a ResizeObserver (a removed element's box goes to 0). */
  watch?: (canvas: HTMLCanvasElement, changed: () => void) => () => void;
};

function observeSize(canvas: HTMLCanvasElement, changed: () => void): () => void {
  const Observer = canvas.ownerDocument?.defaultView?.ResizeObserver;
  if (!Observer) return () => {};
  const observer = new Observer(changed);
  observer.observe(canvas);
  return () => observer.disconnect();
}

/** The resolution menu's enum for today's quality factor. */
function resolution(quality: number): ViewParams['resolution'] {
  if (!(quality > 0) || quality >= 0.75) return 'full';
  return quality >= 0.375 ? 'half' : 'quarter';
}

export class EngineCanvasBinding {
  private canvas: HTMLCanvasElement | null = null;
  private presentation: BoundPresentation | null = null;
  private stage: HTMLElement | null = null;
  /** The binding hid the passed canvas (and shows it again when it ends). */
  private hid = false;
  /** Times a bound presentation was made (diagnostics, tests). */
  made = 0;

  constructor(private readonly options: CanvasBindingOptions) {}

  /** The canvas engine presentation is bound to, if any. */
  get bound(): HTMLCanvasElement | null {
    return this.canvas;
  }

  /** Whether a presentation of the binding's own is up. */
  get presenting(): boolean {
    return this.presentation !== null;
  }

  /** Whether a viewer of its own presents over `canvas` (the built-in one),
   *  or is mounted to (it presents once the engine connection is up). */
  private owned(canvas: HTMLCanvasElement): boolean {
    return !!canvas.closest(`[data-engine-stage="viewer"], [${ENGINE_SELF_PRESENTS_ATTRIBUTE}]`);
  }

  /**
   * `render.gl.init(canvas)`: binds engine presentation to `canvas`. The
   * presentation is made at the first `resize` (when the caller has placed
   * the canvas). Returns whether the engine presents for this canvas.
   */
  bind(canvas: HTMLCanvasElement): boolean {
    if (!this.options.service()) return false;
    if (this.canvas !== canvas) this.unbind();
    this.canvas = canvas;
    const doc = canvas.ownerDocument;
    if (doc && this.watching !== doc) {
      this.stopWatching();
      // A viewer that starts presenting by itself over this canvas wins at
      // once, whether or not it resizes the canvas again.
      const yieldToViewer = (): void => {
        if (this.canvas && this.owned(this.canvas)) this.drop(false);
      };
      doc.addEventListener(ENGINE_STAGE_EVENT, yieldToViewer);
      this.watching = doc;
      this.unwatch = () => doc.removeEventListener(ENGINE_STAGE_EVENT, yieldToViewer);
    }
    return true;
  }

  private watching: Document | null = null;
  private unwatch: (() => void) | null = null;
  /** Stops watching the bound canvas leave the document. */
  private unwatchCanvas: (() => void) | null = null;

  private stopWatching(): void {
    this.unwatch?.();
    this.unwatch = null;
    this.watching = null;
  }

  /** `render.gl.resize(width, height, previewViewport)`: the canvas moved
   *  or changed size; the engine follows. */
  resize(): void {
    const canvas = this.canvas;
    if (!canvas) return;
    if (!canvas.isConnected) {
      this.unbind();
      return;
    }
    if (this.owned(canvas)) {
      // The built-in viewer presents by itself: no second presenter (and
      // the canvas's visibility is the viewer's).
      this.drop(false);
      return;
    }
    const service = this.options.service();
    const stage = canvas.parentElement;
    const comp = this.options.composition();
    if (!service || !stage || !comp || !(comp.w > 0 && comp.h > 0)) return;
    if (this.presentation && this.stage !== stage) this.drop();
    if (!this.presentation) {
      this.stage = stage;
      this.presentation = this.options.present({ service, stage, before: canvas.nextSibling as Element | null, role: 'binding' });
      this.made++;
      this.unwatchCanvas = (this.options.watch ?? observeSize)(canvas, () => {
        if (this.canvas === canvas && !canvas.isConnected) this.unbind();
      });
      if (this.presentation.exclusive) {
        canvas.style.visibility = 'hidden';
        this.hid = true;
      }
    }
    const parent = stage.getBoundingClientRect();
    const box = canvas.getBoundingClientRect();
    const zoom = box.width > 0 ? box.width / comp.w : 1;
    const pan: [number, number] = [
      box.left + box.width / 2 - (parent.left + parent.width / 2),
      box.top + box.height / 2 - (parent.top + parent.height / 2)
    ];
    this.presentation.update(
      { zoom: zoom > 0 && Number.isFinite(zoom) ? zoom : 1, pan, resolution: resolution(this.options.quality()), media: 'automatic', playEveryFrame: false },
      { width: parent.width, height: parent.height }
    );
  }

  /** Ends the binding's presentation (the canvas stays bound). `restore`:
   *  show the canvas it hid (not when a viewer took the canvas over). */
  private drop(restore = true): void {
    this.unwatchCanvas?.();
    this.unwatchCanvas = null;
    this.presentation?.dispose();
    this.presentation = null;
    this.stage = null;
    if (restore && this.hid && this.canvas) this.canvas.style.visibility = '';
    this.hid = false;
  }

  /** Forgets the canvas and its presentation. */
  unbind(): void {
    this.drop();
    this.stopWatching();
    this.canvas = null;
  }
}
