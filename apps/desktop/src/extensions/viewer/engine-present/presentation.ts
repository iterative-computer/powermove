/*
 * The viewer's engine presentation, behind "Use New Engine" (plan section
 * 18; amendments 4 and 5).
 *
 * The kernel's engine connection registers `engine.viewer`
 * (`EngineViewerService`) only when the new engine is on. When the viewer
 * finds it, it attaches a `ViewerPresentation` to its stage:
 *
 * - a canvas over the stage (below the DOM and Canvas2D overlay), drawn
 *   with WebGPU from the engine's shared surfaces (`EnginePresenter`);
 * - a viewer session on the interactive engine (`open`): the viewer state
 *   (size in device pixels, DPR, zoom, pan, resolution, media preference,
 *   canvas color space, EDR, the view override, Play Every Frame), numbered
 *   by `ViewSequencer`, goes to the engine whenever it changes; descriptors,
 *   surface pools, and clock samples come back on the same port;
 * - the frame tags (`revision`, `viewSeq`) and overlay geometry of every
 *   frame drawn, for overlays that must not lead the pixels
 *   (`onPresented`, `overlayInCss`).
 *
 * Following the window. The presentation loop runs on the animation frames
 * of whichever window currently hosts the stage, and the display's EDR
 * capability, color gamut, and DPR come from that window. When the stage
 * moves to a popout (or back) the loop moves with it, the display timing
 * refits, and the new viewer state (new DPR, EDR) makes the engine replace
 * the pool. The receiver stays in the main window's preload: surfaces are
 * sent to the window that owns the engine port, and same-origin popouts
 * draw from the same realm.
 *
 * The canvas stays hidden until the engine presents a frame for this
 * session; until then (and whenever the engine restarts) the current
 * engine's canvas underneath is what the viewer shows.
 */
import { createWebGpuPainter, type FramePainter } from './gpu';
import { EnginePresenter, type FrameLink, type FrameSource, type PresentedFrame, type PresenterStats } from './presenter';
import type { EngineFrameMessage, OverlayGeometry, PathCommand, Quad, ViewerState } from './protocol';
import { canvasColorSpace, displayIsEdr, ViewSequencer, type ViewInput } from './view-state';

/** The service name the kernel's engine connection registers. */
export const ENGINE_VIEWER_SERVICE = 'engine.viewer';

/** One viewer session on the interactive engine. */
export interface EngineViewerChannel extends FrameLink {
  /** Ends the session (the engine retires its pool). */
  close(): void;
}

export type EngineViewerHandlers = {
  /** A frame descriptor, surface pool, or clock sample for this session,
   *  decoded, with its arrival time (performance.now()). */
  message(message: EngineFrameMessage, arrivedMs: number): void;
  /** The session ended (the engine restarted or its port closed). The
   *  viewer opens a new one after `onReady`. */
  closed(reason: string): void;
};

/** What the kernel's engine connection provides to the viewer. */
export interface EngineViewerService {
  /** The preload's frame bridge receiver for this window. */
  readonly source: FrameSource;
  /** The engine id of the composition the viewer shows. */
  composition(): string;
  /** Opens a viewer session; null while the interactive engine is not connected. */
  open(handlers: EngineViewerHandlers): EngineViewerChannel | null;
  /** Called whenever `open` may succeed (the engine connected or reconnected). */
  onReady(listener: () => void): () => void;
  /** performance.now() ms to host µs, when the connection measured it. */
  hostOffsetUs?(): number | null;
  /** The refresh rate of the display showing `view`, when main knows it. */
  displayHz?(view: Window): number | null;
}

/** What the viewer decides; the presentation adds what the window knows. */
export type ViewParams = Omit<ViewInput, 'composition' | 'cssWidth' | 'cssHeight' | 'devicePixelRatio' | 'edr' | 'colorSpace'>;

export type PresentationOptions = {
  service: EngineViewerService;
  /** The element the canvas covers (the viewer's stage). */
  stage: HTMLElement;
  /** The canvas goes before this child of the stage (the overlay canvas). */
  before?: Element | null;
  createPainter?: (canvas: HTMLCanvasElement) => Promise<FramePainter>;
  /** The DPR cap (the viewer renders at most 2x). */
  maxDpr?: number;
  /**
   * A size change reaches the engine once the size has held still this
   * long (`SIZE_SETTLE_MS`); until then frames keep the pool's size and the
   * canvas stretches. Every size is a new pool of up to 32 surfaces, so a
   * live window resize must not replace the pool at every step. 0 applies
   * sizes at once.
   */
  sizeSettleMs?: number;
  /** Timers for the size settle (tests); the hosting window's otherwise. */
  timers?: { set(fn: () => void, ms: number): number; clear(id: number): void };
  /** Records per-frame timings (the S1 measurement harness). */
  trace?: boolean;
  /** `off`: present 8-bit (`bgra` with dither) even on an EDR display
   *  (a setting, and the harness's comparison); `auto` follows the display. */
  edr?: 'auto' | 'off';
  log?: (level: 'info' | 'warn' | 'error', message: string) => void;
};

/** Recent viewer states by sequence, so overlay geometry converts with the
 *  DPR of the state its frame shows. */
const STATE_HISTORY = 32;

/** How long a new viewer size must hold before the engine gets it. */
export const SIZE_SETTLE_MS = 150;

export class ViewerPresentation {
  readonly canvas: HTMLCanvasElement;
  private painter: FramePainter | null = null;
  private painterFailed = false;
  private presenter: EnginePresenter | null = null;
  private channel: EngineViewerChannel | null = null;
  private readonly sequencer = new ViewSequencer();
  private readonly states: ViewerState[] = [];
  private params: ViewParams | null = null;
  private host: Window | null = null;
  private mediaOffs: (() => void)[] = [];
  private latestFrame: PresentedFrame | null = null;
  private readonly listeners = new Set<(frame: PresentedFrame) => void>();
  private readonly offReady: () => void;
  private sessions = 0;
  private disposed = false;
  /** The CSS size the engine last got, and the pending settle. */
  private appliedSize: { width: number; height: number } | null = null;
  private settleTimer = 0;
  private settleDue = false;

  constructor(private readonly options: PresentationOptions) {
    const doc = options.stage.ownerDocument;
    this.canvas = doc.createElement('canvas');
    this.canvas.id = 'engine-present';
    this.canvas.setAttribute('aria-hidden', 'true');
    this.canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;pointer-events:none;visibility:hidden';
    const before = options.before && options.before.parentElement === options.stage ? options.before : null;
    options.stage.insertBefore(this.canvas, before);
    this.offReady = options.service.onReady(() => void this.open());
    void this.open();
  }

  /** Whether engine frames are on screen now. */
  get presenting(): boolean {
    return this.canvas.style.visibility === 'visible';
  }

  /** The newest frame drawn, with its tags and overlay geometry. */
  get latest(): PresentedFrame | null {
    return this.latestFrame;
  }

  get state(): ViewerState | null {
    return this.sequencer.current;
  }

  stats(): (PresenterStats & { sessions: number; viewSeq: number }) | null {
    const stats = this.presenter?.stats;
    return stats ? { ...stats, sessions: this.sessions, viewSeq: this.sequencer.current?.viewSeq ?? 0 } : null;
  }

  onPresented(listener: (frame: PresentedFrame) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /**
   * The viewer's layout changed (size, zoom, pan, resolution, view
   * override) or the window did. Sends the state when it differs from the
   * last one.
   */
  update(params: ViewParams): void {
    if (this.disposed) return;
    this.params = params;
    const view = this.follow();
    if (!view) return;
    const rect = this.options.stage.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) return;
    const size = this.settledSize(view, rect.width, rect.height);
    const dpr = Math.min(view.devicePixelRatio || 1, this.options.maxDpr ?? 2);
    const { state, changed } = this.sequencer.next({
      ...params,
      composition: this.options.service.composition(),
      cssWidth: size.width,
      cssHeight: size.height,
      devicePixelRatio: dpr,
      edr: this.options.edr !== 'off' && displayIsEdr(view),
      colorSpace: canvasColorSpace(view)
    });
    if (!changed) return;
    this.states.push(state);
    if (this.states.length > STATE_HISTORY) this.states.shift();
    this.presenter?.setViewerState(state);
  }

  /** The size to send: a new size only once it held still (live resizes). */
  private settledSize(view: Window, width: number, height: number): { width: number; height: number } {
    const applied = this.appliedSize;
    const settleMs = this.options.sizeSettleMs ?? SIZE_SETTLE_MS;
    if (!applied || this.settleDue || settleMs <= 0 || (applied.width === width && applied.height === height)) {
      this.settleDue = false;
      if (this.settleTimer) this.clearTimer(view, this.settleTimer);
      this.settleTimer = 0;
      this.appliedSize = { width, height };
      return this.appliedSize;
    }
    if (this.settleTimer) this.clearTimer(view, this.settleTimer);
    const fire = (): void => {
      this.settleTimer = 0;
      this.settleDue = true;
      if (this.params) this.update(this.params);
    };
    this.settleTimer = this.options.timers ? this.options.timers.set(fire, settleMs) : view.setTimeout(fire, settleMs);
    return applied;
  }

  private clearTimer(view: Window, id: number): void {
    if (this.options.timers) this.options.timers.clear(id);
    else view.clearTimeout(id);
  }

  /** The per-frame trace of the current session (the S1 measurement harness). */
  takeTrace(): import('./presenter').TraceRecord | null {
    const trace = this.presenter?.trace ?? null;
    if (!trace) return null;
    const copy = { arrivals: trace.arrivals.splice(0), draws: trace.draws.splice(0), drops: trace.drops.splice(0), releases: trace.releases.splice(0), rafs: trace.rafs.splice(0) };
    return copy;
  }

  /** The viewer state a frame shows (by its view sequence). */
  stateFor(viewSeq: number): ViewerState | null {
    for (let i = this.states.length - 1; i >= 0; i--) if (this.states[i]!.viewSeq === viewSeq) return this.states[i]!;
    return null;
  }

  /** A frame's overlay geometry in the stage's CSS pixels. */
  overlayInCss(frame: PresentedFrame | null = this.latestFrame): OverlayGeometry | null {
    if (!frame?.overlay) return null;
    const dpr = this.stateFor(frame.viewSeq)?.devicePixelRatio ?? this.sequencer.current?.devicePixelRatio ?? 1;
    return overlayToCss(frame.overlay, dpr);
  }

  /** Moves the loop and the display queries to the window hosting the stage. */
  private follow(): Window | null {
    const view = this.options.stage.ownerDocument.defaultView;
    if (!view || view === this.host) return view;
    this.host = view;
    for (const off of this.mediaOffs.splice(0)) off();
    const refresh = (): void => {
      if (this.params) this.update(this.params);
    };
    // A popout is sized after it opens: its resizes count as layout.
    view.addEventListener?.('resize', refresh);
    this.mediaOffs.push(() => view.removeEventListener?.('resize', refresh));
    for (const query of ['(dynamic-range: high)', '(color-gamut: p3)', `(resolution: ${view.devicePixelRatio || 1}dppx)`]) {
      const list = view.matchMedia?.(query);
      if (!list?.addEventListener) continue;
      list.addEventListener('change', refresh);
      this.mediaOffs.push(() => list.removeEventListener('change', refresh));
    }
    // Every window has its own time origin: a popout's rAF timestamps are
    // moved onto this realm's performance.now(), the clock the presenter
    // stamps arrivals with and maps to host time.
    const shift = timeOriginShift(view);
    this.presenter?.retarget(
      (cb) => view.requestAnimationFrame((t) => cb(t + shift)),
      (id) => view.cancelAnimationFrame(id),
      this.options.service.displayHz?.(view) ?? null
    );
    return view;
  }

  private async open(): Promise<void> {
    if (this.disposed || this.channel) return;
    if (!this.painter) {
      if (this.painterFailed) return;
      try {
        this.painter = await (this.options.createPainter ?? createWebGpuPainter)(this.canvas);
      } catch (error) {
        this.painterFailed = true;
        this.options.log?.('error', `engine presentation is unavailable: ${error instanceof Error ? error.message : String(error)}`);
        return;
      }
      if (this.disposed) {
        this.painter.dispose();
        return;
      }
      if (this.channel) return;
    }
    let presenter: EnginePresenter | null = null;
    let opened: EngineViewerChannel | null = null;
    const channel = this.options.service.open({
      message: (message, arrivedMs) => presenter?.receive(message, arrivedMs),
      closed: (reason) => this.closed(opened, reason)
    });
    if (!channel) return;
    opened = channel;
    this.channel = channel;
    this.sessions++;
    presenter = new EnginePresenter({
      link: channel,
      source: this.options.service.source,
      painter: this.painter,
      trace: this.options.trace,
      hostOffsetUs: this.options.service.hostOffsetUs ? () => this.options.service.hostOffsetUs!() : undefined
    });
    this.presenter = presenter;
    presenter.onPresented((frame) => {
      this.latestFrame = frame;
      if (this.canvas.style.visibility !== 'visible') this.canvas.style.visibility = 'visible';
      for (const listener of this.listeners) listener(frame);
    });
    this.host = null;
    const view = this.follow();
    if (!view) presenter.start();
    // A new session knows nothing yet: send the current state.
    const state = this.sequencer.current;
    if (state) presenter.setViewerState(state);
    else if (this.params) this.update(this.params);
  }

  private closed(channel: EngineViewerChannel | null, reason: string): void {
    if (!channel || channel !== this.channel) return;
    this.options.log?.('info', `engine viewer session ended: ${reason}`);
    this.channel = null;
    this.presenter?.dispose();
    this.presenter = null;
    this.latestFrame = null;
    this.canvas.style.visibility = 'hidden';
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.settleTimer && this.host) this.clearTimer(this.host, this.settleTimer);
    this.settleTimer = 0;
    this.offReady();
    for (const off of this.mediaOffs.splice(0)) off();
    this.presenter?.dispose();
    this.presenter = null;
    this.channel?.close();
    this.channel = null;
    this.painter?.dispose();
    this.painter = null;
    this.listeners.clear();
    this.canvas.remove();
  }
}

/** Milliseconds to add to `view`'s performance.now() to read this realm's. */
export function timeOriginShift(view: Window): number {
  const theirs = view.performance?.timeOrigin;
  const ours = globalThis.performance?.timeOrigin;
  return typeof theirs === 'number' && typeof ours === 'number' && Number.isFinite(theirs - ours) ? theirs - ours : 0;
}

const scalePoint = (p: [number, number], s: number): [number, number] => [p[0] * s, p[1] * s];

function scalePath(command: PathCommand, s: number): PathCommand {
  switch (command.op) {
    case 'moveTo':
    case 'lineTo':
      return { op: command.op, x: command.x * s, y: command.y * s };
    case 'cubicTo':
      return { op: 'cubicTo', x1: command.x1 * s, y1: command.y1 * s, x2: command.x2 * s, y2: command.y2 * s, x: command.x * s, y: command.y * s };
    case 'close':
      return command;
  }
}

/**
 * Overlay geometry from frame (device) pixels to CSS pixels: rectangles,
 * bounds, paths, and the caret scale by 1/DPR; a 2D world matrix (column
 * major 3×3, composition pixels to frame pixels) is premultiplied by the
 * same scale. 3D matrices map to clip space and are kept.
 */
export function overlayToCss(overlay: OverlayGeometry, dpr: number): OverlayGeometry {
  const s = dpr > 0 && Number.isFinite(dpr) ? 1 / dpr : 1;
  return {
    viewport: { x: overlay.viewport.x * s, y: overlay.viewport.y * s, width: overlay.viewport.width * s, height: overlay.viewport.height * s },
    layers: overlay.layers.map((layer) => ({
      layer: layer.layer,
      world2d: layer.world2d ? layer.world2d.map((v, i) => (i % 3 === 2 ? v : v * s)) : null,
      world3d: layer.world3d,
      bounds: layer.bounds.map((p) => scalePoint(p, s)) as Quad,
      paths: layer.paths.map((path) => path.map((c) => scalePath(c, s)))
    })),
    caret: overlay.caret ? { top: scalePoint(overlay.caret.top, s), bottom: scalePoint(overlay.caret.bottom, s) } : null
  };
}
