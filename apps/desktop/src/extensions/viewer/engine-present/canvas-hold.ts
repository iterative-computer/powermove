/**
 * Today's gesture hold for the engine's viewer canvas (decision V1).
 *
 * During a paused zoom or pan gesture, today's viewer keeps its committed
 * WebGL canvas, and the browser scales its last frame into the new CSS box,
 * until the gesture settles (`zoomGestureUntil`, 100 ms after the latest
 * event). `canRetainNavigation` in viewer.ts decides it on every layout.
 * It holds while all of these are true:
 * - the layout comes from a gesture that has not settled;
 * - nothing plays;
 * - the frame on screen is current (`presentationIsCurrent`): the
 *   project object, its size, the animation version, the time and the
 *   quality it was drawn at are today's;
 * - the visible part of the composition stays inside the canvas's crop
 *   (when it is cropped).
 * Any other layout commits today's canvas for the new view. After the
 * gesture, the next frame loop tick commits it (`deferNavigationRender`).
 *
 * With "Use New Engine" on, today's canvas never resizes (the compositor
 * rests), so the viewer keeps the engine's committed canvas by the same
 * rule. Here the frame on screen is the engine's: it is current when it
 * was drawn at the committed canvas, at the document revision the engine
 * has now, at today's time, and is not a scrub stand-in. When the
 * gesture settles, the hold lays the viewer out again so the engine
 * follows (today's frame loop does that for the compositor).
 */
import type { ViewerCanvas } from './protocol';

/** The composition area a view shows, in composition pixels. */
export type VisibleRegion = { x: number; y: number; right: number; bottom: number };

/** The crop fields of today's `PreviewViewport`. */
export type CanvasCrop = { x: number; y: number; width: number; height: number };

/** A canvas today's viewer committed, and what it was committed for. */
export type CommittedCanvas<Crop extends CanvasCrop = CanvasCrop> = {
  /** Backing size (`previewRenderSize`, the explicit resolution, or the crop's render size). */
  render: { width: number; height: number };
  /** `previewRenderViewport`'s crop, or null for the whole composition. */
  viewport: Crop | null;
  project: unknown;
  compWidth: number;
  compHeight: number;
  version: number;
  time: number;
  quality: number;
};

/** The engine's frame on screen: the canvas it was drawn at (the viewer
 *  state its `viewSeq` names), and whether its content is today's. */
export type EngineFrameOnScreen = { canvas: ViewerCanvas | null | undefined; current: boolean } | null;

export type HoldInput<Crop extends CanvasCrop = CanvasCrop> = {
  /** The layout comes from a zoom or pan gesture. */
  gesture: boolean;
  now: number;
  /** `zoomGestureUntil`: when the gesture settles. */
  gestureUntil: number;
  playing: boolean;
  /** Today's canvas for this layout. */
  next: CommittedCanvas<Crop>;
  visible: VisibleRegion;
  onScreen: EngineFrameOnScreen;
};

/** Whether the engine's frame on screen was drawn at `committed`. */
export function drawnAt(canvas: ViewerCanvas | null | undefined, committed: CommittedCanvas): boolean {
  if (!canvas) return false;
  const crop = committed.viewport ?? { x: 0, y: 0, width: committed.compWidth, height: committed.compHeight };
  return canvas.width === Math.round(committed.render.width) && canvas.height === Math.round(committed.render.height)
    && canvas.crop.x === crop.x && canvas.crop.y === crop.y
    && canvas.crop.width === crop.width && canvas.crop.height === crop.height;
}

/** `canRetainNavigation` for the engine: keep `committed` for this layout. */
export function holdsCommittedCanvas<Crop extends CanvasCrop>(committed: CommittedCanvas<Crop> | null, input: HoldInput<Crop>): committed is CommittedCanvas<Crop> {
  if (!input.gesture || !committed || input.now >= input.gestureUntil || input.playing) return false;
  const next = input.next;
  if (committed.project !== next.project || committed.compWidth !== next.compWidth || committed.compHeight !== next.compHeight
    || committed.version !== next.version || committed.time !== next.time || committed.quality !== next.quality) return false;
  if (!input.onScreen?.current || !drawnAt(input.onScreen.canvas, committed)) return false;
  const crop = committed.viewport, visible = input.visible;
  if (!crop) return true;
  return visible.x >= crop.x && visible.y >= crop.y
    && visible.right <= crop.x + crop.width && visible.bottom <= crop.y + crop.height;
}

export type HoldTimers = {
  setTimeout(fn: () => void, ms: number): number;
  clearTimeout(id: number): void;
  now(): number;
  /** `zoomGestureUntil` now (later events of the gesture move it). */
  gestureUntil(): number;
  /** Lays the viewer out again (not as a gesture). */
  relayout(): void;
};

/** The engine's committed canvas through gestures, and the settle that lets it follow. */
export class EngineCanvasHold<Crop extends CanvasCrop = CanvasCrop> {
  private kept: CommittedCanvas<Crop> | null = null;
  private timer = 0;

  constructor(private readonly timers: HoldTimers) {}

  /** The canvas committed for the engine (what today's `GL.previewViewport` is to the compositor). */
  get committed(): CommittedCanvas<Crop> | null {
    return this.kept;
  }

  /** Whether a settle is waiting for the gesture to end. */
  get settling(): boolean {
    return this.timer !== 0;
  }

  /** The canvas the engine renders this layout at: the committed one while
   *  the gesture holds it (a settle follows), else `input.next`, committed. */
  layout(input: HoldInput<Crop>): CommittedCanvas<Crop> {
    if (holdsCommittedCanvas(this.kept, input)) {
      this.settle();
      return this.kept;
    }
    this.cancel();
    this.kept = input.next;
    return input.next;
  }

  private settle(): void {
    if (this.timer) return;
    const run = () => {
      const left = this.timers.gestureUntil() - this.timers.now();
      if (left > 0) {
        this.timer = this.timers.setTimeout(run, left + 1);
        return;
      }
      this.timer = 0;
      this.timers.relayout();
    };
    this.timer = this.timers.setTimeout(run, Math.max(1, this.timers.gestureUntil() - this.timers.now() + 1));
  }

  private cancel(): void {
    if (this.timer) this.timers.clearTimeout(this.timer);
    this.timer = 0;
  }

  dispose(): void {
    this.cancel();
    this.kept = null;
  }
}
