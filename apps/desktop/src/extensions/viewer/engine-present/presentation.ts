import { ChromeLeases } from './chrome-leases';
/*
 * The viewer's engine presentation, behind "Use New Engine" (plan section
 * 18; amendments 4 and 5).
 *
 * The kernel's engine connection registers `engine.viewer`
 * (`EngineViewerService`) only when the new engine is on. When the viewer
 * finds it, it attaches a `ViewerPresentation` to its stage:
 *
 * - a canvas over the stage (below the DOM and Canvas2D overlay), drawn
 *   with WebGL2 from the engine's shared surfaces (`EnginePresenter`);
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
import type { FramePainter } from './gpu';
import { createFramePainter } from './painter';
import { EnginePresenter, type FrameLink, type FrameSource, type PresentedFrame, type Presenter, type PresenterStats, type TraceRecord } from './presenter';
import { PresentWorker, type WorkerLike } from './worker-presenter';
import type { PresentationWarmup } from './prewarm';
import type { EngineFrameMessage, EngineRouteMessage, OverlayGeometry, PathCommand, Quad, ViewerState, VsyncHint } from './protocol';
import { canvasColorSpace, displayIsEdr, ViewSequencer, type ViewInput } from './view-state';

/** The service name the kernel's engine connection registers. */
export const ENGINE_VIEWER_SERVICE = 'engine.viewer';

/**
 * The window event the kernel's engine connection dispatches right after it
 * registers or disposes `engine.viewer`. The services API has no change
 * notification (and `extensions:changed` would refresh unrelated UI), so
 * this is how a viewer that is already open attaches or detaches at once
 * instead of at its next layout:
 *
 *   api.services.register(ENGINE_VIEWER_SERVICE, service);
 *   window.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
 */
export const ENGINE_VIEWER_CHANGED = 'powermove:engine-viewer-changed';

/** One viewer session on the interactive engine. */
export interface EngineViewerChannel extends FrameLink {
  /** Detaches this viewer. */
  close(): void;
  /** The built-in channel may retire this viewer's imports after GPU and
   * chrome readers drain. Custom channels can omit this capability. */
  retireClosedPool?(generation: number): number;
}

export type EngineViewerHandlers = {
  /** A frame descriptor, surface pool, or clock sample for this session,
   *  decoded, with its arrival time (performance.now()); or the engine's
   *  routing of the composition to the current engine. */
  message(message: EngineFrameMessage | EngineRouteMessage, arrivedMs: number): void;
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
  /** The document revision the engine's frames are tagged with, now (the
   *  overlay draws the geometry of the revision on screen). */
  revision?(): number | null;
  /** The refresh rate of the display showing `view`, when main knows it. */
  displayHz?(view: Window): number | null;
  /** Record per-frame timings in the viewer's presentation (the S1
   *  measurement harness's connection sets it; the kernel's never does). */
  readonly trace?: boolean;
  /** `false`: draw engine frames on the page's main thread rather than in
   *  a presentation worker (the measurement harness compares the two). */
  readonly offMainThread?: boolean;
  /** One detached drawing prepared by the native connection before mount. */
  readonly drawingWarmup?: PresentationWarmup;
  /**
   * "Use New Engine" is on (the kernel's connection): engine frames are the
   * only frames this viewer shows. Until the first frame, while the engine
   * restarts, and after the GPU fails, the last frame stays (or the stage
   * stays empty) with a visible state; nothing hands the viewer back to
   * the current engine.
   */
  readonly exclusive?: boolean;
  /** Each new frame on screen, with the quality it was rendered at (the
   *  kernel emits `frame:rendered` and `overlay` from it). */
  presented?(frame: PresentedFrame, quality: number): void;
  /** Animation frames a canvas that was hidden waits after its first frame
   *  before that frame counts as on screen, and for a worker's frame until
   *  the canvas took its size (default 2: the drawing's canvas and the page
   *  commit separately, so a frame just submitted is not shown yet). 0
   *  counts it at once (tests that run frames by hand). */
  readonly revealFrames?: number;
}

/** Longest a newly shown canvas waits for its first frame to land (animation frames). */
export const MAX_REVEAL_FRAMES = 30;

/** Dispatched (bubbling) by a viewer's own presentation on its stage. */
export const ENGINE_STAGE_EVENT = 'powermove:engine-stage';

/**
 * The stage attribute (dataset key `engineSelfPresents`) of a viewer that
 * presents engine frames by itself, set for as long as it is mounted, so a
 * `render.gl.init` binding never presents over its canvas, before its own
 * presentation exists (the engine connection comes up after the viewer
 * mounts) and after it is gone (the built-in viewer's WebGL canvas outlives
 * it in the panel skeleton).
 */
export const ENGINE_SELF_PRESENTS_ATTRIBUTE = 'data-engine-self-presents';

/** The visible states of an exclusive presentation without a frame to show. */
export const PRESENTATION_STATUS = {
  starting: 'Starting the engine…',
  reconnecting: 'Reconnecting to the engine…',
  gpuFailed: 'The viewer cannot draw: the GPU keeps failing. Restart Powermove.',
  unreadable: 'The viewer cannot read the engine’s frames. Restart Powermove.',
  unavailable: 'The viewer cannot draw engine frames:',
  unsupported: 'The new engine cannot draw this composition yet.'
} as const;

/** The stage's status for a composition the engine cannot draw: the
 *  engine's sentence (it names what), or a generic one without it. */
export function routedStatus(reason: string | null | undefined): string {
  const text = (reason ?? '').trim();
  return text || PRESENTATION_STATUS.unsupported;
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
  /** `viewer`: the viewer presents by itself (the default); `binding`: the
   *  kernel's `render.gl.init` binding presents for a viewer that does not
   *  (engine-connection/canvas-binding.ts). The stage is marked with it
   *  (`data-engine-stage`), so the two never present over one stage. */
  role?: 'viewer' | 'binding';
  /** Timers for the size settle (tests); the hosting window's otherwise. */
  timers?: { set(fn: () => void, ms: number): number; clear(id: number): void };
  /** Records per-frame timings (the S1 measurement harness). */
  trace?: boolean;
  /** `off`: present 8-bit (`bgra` with dither) even on an EDR display
   *  (a setting, and the harness's comparison); `auto` follows the display. */
  edr?: 'auto' | 'off';
  log?: (level: 'info' | 'warn' | 'error', message: string) => void;
  /**
   * Starts a presentation worker (present-worker-entry.ts). With it the
   * canvas hands its drawing to the worker, which presents engine frames
   * on its own animation frames, independent of this thread's stalls
   * (present-worker.ts). Without it, or when the worker cannot draw,
   * frames are drawn on this thread.
   */
  createWorker?: () => WorkerLike;
};

/** Recent viewer states by sequence, so overlay geometry converts with the
 *  DPR of the state its frame shows. */
const STATE_HISTORY = 32;

/** How long a new viewer size must hold before the engine gets it. */
export const SIZE_SETTLE_MS = 150;

/** Device losses tolerated within `DEVICE_LOSS_WINDOW_MS` before the presentation gives up. */
export const MAX_DEVICE_LOSSES = 3;
export const DEVICE_LOSS_WINDOW_MS = 30_000;

/**
 * Surface renewals (`renewSurfaces`) allowed within `DEVICE_LOSS_WINDOW_MS`:
 * up to two per tolerated device loss (one when the drawing is replaced,
 * one more if the first pool was imported while the GPU process was still
 * coming back). Beyond it the stage says the frames cannot be read, until
 * the window passes or a frame draws.
 */
export const MAX_SURFACE_RENEWALS = 2 * MAX_DEVICE_LOSSES;

export class ViewerPresentation {
  private canvasElement: HTMLCanvasElement;
  /** Draws on this thread (`painter`) or in a presentation worker (`worker`). */
  private painter: FramePainter | null = null;
  private painterFailed = false;
  private drawingPending: { token: number; promise: Promise<boolean> } | null = null;
  private worker: PresentWorker | null = null;
  /** The worker could not present: stay on this thread. */
  private workerUnavailable = false;
  private presenter: Presenter | null = null;
  private channel: EngineViewerChannel | null = null;
  private chromeLeases: ChromeLeases | null = null;
  /** Imports survive presenter replacement, but no longer than this viewer.
   * Keep each generation's lease through a reconnect: chrome reads of the
   * retained last frame can outlive its old channel. */
  private readonly poolLeases = new Map<number, Set<ChromeLeases | null>>();
  private readonly draining = new Set<Promise<void>>();
  private readonly sequencer = new ViewSequencer();
  private readonly states: ViewerState[] = [];
  private params: ViewParams | null = null;
  private host: Window | null = null;
  /** Keep the configured surface colors when a host's capability queries change. */
  private displayColor: {
    host: Window;
    colorSpace: ViewerState['colorSpace'];
    edr: boolean;
    observedColorSpace: ViewerState['colorSpace'];
    observedEdr: boolean;
  } | null = null;
  private mediaOffs: (() => void)[] = [];
  private latestFrame: PresentedFrame | null = null;
  /** The frame `seq` at or below which frames are stale since the engine
   *  said it cannot draw the composition (`routed`): the newest seq the
   *  engine had given the view when it decided (`message.seq`), and at
   *  least the newest the channel had delivered (frames still in the
   *  presenter). A frame above it is a later decision even at the same
   *  revision (a composition switch: the document batch reaches the engine
   *  before the viewer state). Arrival order alone does not decide it:
   *  paused frames leave from the engine's completion thread, so a frame
   *  drawn before the report can arrive after it with a higher seq than
   *  any delivered so far. */
  private routedSeq: number | null = null;
  /** The newest frame `seq` the current channel delivered. */
  private deliveredSeq = 0;
  private readonly listeners = new Set<(frame: PresentedFrame) => void>();
  private readonly hiddenListeners = new Set<() => void>();
  private readonly offReady: () => void;
  private sessions = 0;
  private disposed = false;
  /** Pools and clock samples that came while no presenter existed. */
  private held: { message: EngineFrameMessage; arrivedMs: number }[] = [];
  /** Bumped whenever the canvas and its drawing are replaced. */
  private backendToken = 0;
  /** The CSS size the engine last got, and the pending settle. */
  private appliedSize: { width: number; height: number } | null = null;
  private settleTimer = 0;
  private pendingSize: { width: number; height: number } | null = null;
  private settleDue = false;
  /** The display's refresh grid, sent with every state (amendment 18). */
  private vsync: VsyncHint | null = null;
  /** When the drawing's device was lost recently (performance.now ms). */
  private losses: number[] = [];
  /** When the surfaces were renewed recently (performance.now ms). */
  private renewals: number[] = [];
  /** The newest pool generation a renewal replaces: frames of it and older
   *  pools that cannot be read are the failure already being handled. */
  private renewedFrom = 0;

  /** The visible state while an exclusive presentation has no frame. */
  private statusElement: HTMLElement | null = null;

  constructor(private readonly options: PresentationOptions) {
    this.canvasElement = this.makeCanvas(options.stage.ownerDocument);
    const before = options.before && options.before.parentElement === options.stage ? options.before : null;
    options.stage.insertBefore(this.canvasElement, before);
    options.stage.dataset['engineStage'] = options.role ?? 'viewer';
    // A `render.gl.init` binding over a canvas in this stage yields to it.
    if ((options.role ?? 'viewer') === 'viewer') options.stage.dispatchEvent(new Event(ENGINE_STAGE_EVENT, { bubbles: true }));
    if (options.service.exclusive) this.setStatus(PRESENTATION_STATUS.starting);
    this.offReady = options.service.onReady(() => void this.open());
    void this.open();
  }

  /** Whether engine frames are the only frames this viewer shows. */
  get exclusive(): boolean {
    return this.options.service.exclusive === true;
  }

  /** The visible state shown while there is no frame (null: none). */
  get status(): string | null {
    return this.statusElement?.textContent ?? null;
  }

  private setStatus(text: string | null): void {
    if (text === null) {
      this.statusElement?.remove();
      this.statusElement = null;
      return;
    }
    if (!this.statusElement) {
      const el = this.options.stage.ownerDocument.createElement('div');
      el.dataset['engineStatus'] = '';
      el.setAttribute('role', 'status');
      el.style.cssText = 'position:absolute;left:50%;bottom:12px;transform:translateX(-50%);z-index:5;width:max-content;max-width:calc(100% - 24px);box-sizing:border-box;padding:4px 10px;border-radius:var(--r-sm);background-color:color-mix(in srgb,var(--tx) 5%,var(--bg-panel));color:var(--tx-2);font:var(--fs-md) var(--f-ui);text-align:center;pointer-events:none;overflow:hidden;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:3';
      this.options.stage.append(el);
      this.statusElement = el;
    }
    if (this.statusElement.textContent !== text) this.statusElement.textContent = text;
  }

  /** The canvas engine frames are drawn into (replaced when presentation moves to another window from a worker). */
  get canvas(): HTMLCanvasElement {
    return this.canvasElement;
  }

  /** Whether frames are drawn by a presentation worker rather than this thread. */
  get offMainThread(): boolean {
    return this.worker !== null;
  }

  private makeCanvas(doc: Document, canvas = doc.createElement('canvas')): HTMLCanvasElement {
    canvas.id = 'engine-present';
    canvas.setAttribute('aria-hidden', 'true');
    canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;pointer-events:none;visibility:hidden';
    return canvas;
  }

  /** Whether engine frames are on screen now. */
  get presenting(): boolean {
    return this.canvasElement.style.visibility === 'visible' && this.revealing === null && this.latestFrame !== null;
  }

  /** The newest frame drawn, with its tags and overlay geometry. */
  get latest(): PresentedFrame | null {
    return this.latestFrame;
  }

  /**
   * The depth and object-id surface of a frame on screen (plan section 8:
   * `chromePlanesV1`: losslessly packed f32 depth in RGB; paired coverage
   * and visible IDs from coverageFrame). Both surfaces use the frame size.
   * Ask for it with `ViewParams.depthId`. Null when the frame has none or
   * its import has not arrived. Close it after reading; it stays valid
   * while its frame is the one on screen.
   */
  depthFrame(frame: PresentedFrame | null = this.latestFrame): VideoFrame | null {
    const depth = frame?.depthId;
    if (!depth) return null;
    return this.options.service.source.videoFrame(depth.poolGeneration, depth.poolIndex);
  }

  /** Retain this frame's native slot through the overlay's GPU read fence. */
  retainChromeFrame(frame: PresentedFrame): ((done?: Promise<unknown>) => void) | null {
    return this.chromeLeases?.acquire(frame.seq) ?? null;
  }

  /** Unoccluded selection coverage and visible IDs, aligned to `frame`. */
  coverageFrame(frame: PresentedFrame | null = this.latestFrame): VideoFrame | null {
    const depth = frame?.depthId;
    if (depth?.encoding !== 'chromePlanesV1' || depth.coveragePoolIndex == null) return null;
    return this.options.service.source.videoFrame(depth.poolGeneration, depth.coveragePoolIndex);
  }
  onChromeSurface(listener: () => void): () => void {
    return this.options.service.source.onSurface((generation) => {
      if (this.latestFrame?.depthId?.poolGeneration === generation) listener();
    });
  }

  get state(): ViewerState | null {
    return this.sequencer.current;
  }

  stats(): (PresenterStats & { sessions: number; viewSeq: number; offMainThread: boolean }) | null {
    const stats = this.presenter?.stats;
    return stats ? { ...stats, sessions: this.sessions, viewSeq: this.sequencer.current?.viewSeq ?? 0, offMainThread: this.offMainThread } : null;
  }

  onPresented(listener: (frame: PresentedFrame) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /**
   * Engine frames left the screen without `dispose` (the session ended, or
   * the drawing was replaced): what is under the canvas shows again, so the
   * viewer catches up on what it skipped while the engine presented.
   */
  onHidden(listener: () => void): () => void {
    this.hiddenListeners.add(listener);
    return () => this.hiddenListeners.delete(listener);
  }

  /** The newest frame drawn while the canvas waits to show (not on screen yet). */
  private revealing: PresentedFrame | null = null;
  private revealFor: Presenter | null = null;
  private revealToken = 0;

  /**
   * A canvas that became visible shows its first frame only once the GPU
   * finished drawing it (at a cold start the drawing's pipelines compile
   * first) and the display took it: the frame counts as on screen
   * (`latest`, `presenting`, frame events) `revealFrames` animation frames
   * after the drawing said its GPU work finished, at most
   * `MAX_REVEAL_FRAMES` after it was drawn. The newest frame drawn by then
   * is the one shown.
   */
  private reveal(presenter: Presenter): void {
    const token = ++this.revealToken;
    this.revealFor = presenter;
    const view = this.options.stage.ownerDocument.defaultView;
    const done = (): void => {
      if (token !== this.revealToken || this.disposed || this.presenter !== presenter) return;
      const frame = this.revealing;
      this.revealing = null;
      this.revealFor = null;
      if (!frame || this.canvasElement.style.visibility !== 'visible') return;
      if (this.statusElement) this.setStatus(null);
      this.show(frame);
    };
    const least = Math.max(0, Math.floor(this.options.service.revealFrames ?? 2));
    // 0: frames run by hand (tests): a shown frame counts at once.
    if (least === 0) return done();
    const revealed = this.revealing?.seq ?? 0;
    let finished = typeof presenter.onDone !== 'function';
    const offDone = presenter.onDone?.((seq) => {
      if (seq >= revealed) finished = true;
    }) ?? (() => {});
    let since = -1;
    const wait = (count: number): void => {
      if (token !== this.revealToken) return offDone();
      if (finished && since < 0) since = count;
      if ((since >= 0 && count - since >= least) || count >= MAX_REVEAL_FRAMES) {
        offDone();
        return done();
      }
      if (view?.requestAnimationFrame) view.requestAnimationFrame(() => wait(count + 1));
      else setTimeout(() => wait(count + 1), 16);
    };
    wait(0);
  }

  /** A frame is on screen: the newest frame, listeners, and the service. */
  private show(frame: PresentedFrame): void {
    const fresh = this.latestFrame?.seq !== frame.seq || this.latestFrame.revision !== frame.revision || this.latestFrame.viewSeq !== frame.viewSeq;
    this.latestFrame = frame;
    this.chromeLeases?.showing(frame.seq);
    for (const listener of this.listeners) listener(frame);
    if (fresh) this.options.service.presented?.(frame, this.qualityOf(frame));
  }

  /** Hides the canvas; tells `onHidden` listeners when it was showing. An
   *  exclusive presentation never hands the stage back: what is under the
   *  canvas is not another engine's frame to show. */
  private hide(canvas: HTMLCanvasElement, handBack = !this.exclusive, always = false): void {
    const was = canvas.style.visibility === 'visible';
    canvas.style.visibility = 'hidden';
    if (canvas === this.canvasElement) {
      this.revealing = null;
      this.revealFor = null;
      this.revealToken++;
    }
    if ((was || always) && !this.disposed && handBack) for (const listener of this.hiddenListeners) listener();
  }

  /**
   * The viewer's layout changed (size, zoom, pan, resolution, view
   * override) or the window did. Sends the state when it differs from the
   * last one. `stage` is the stage's size when the caller already knows it
   * (the viewer keeps it during gestures): reading it here after the
   * viewer wrote styles would force a synchronous layout of the page in
   * every pointer event.
   */
  update(params: ViewParams, stage?: { width: number; height: number }): void {
    if (this.disposed) return;
    this.params = params;
    const view = this.follow();
    if (!view) return;
    const rect = stage ?? this.options.stage.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) return;
    const size = this.settledSize(view, rect.width, rect.height);
    const dpr = Math.min(view.devicePixelRatio || 1, this.options.maxDpr ?? 2);
    const display = this.configuredDisplay(view);
    const { state, changed } = this.sequencer.next({
      ...params,
      composition: this.options.service.composition(),
      cssWidth: size.width,
      cssHeight: size.height,
      devicePixelRatio: dpr,
      edr: display.edr,
      colorSpace: display.colorSpace
    });
    if (!changed) return;
    this.states.push(state);
    if (this.states.length > STATE_HISTORY) this.states.shift();
    this.presenter?.setViewerState(this.withHint(state));
  }

  private configuredDisplay(host: Window): NonNullable<ViewerPresentation['displayColor']> {
    const colorSpace = canvasColorSpace(host);
    const edr = displayIsEdr(host);
    if (!this.displayColor || this.displayColor.host !== host) {
      this.displayColor = { host, colorSpace, edr: this.options.edr !== 'off' && edr, observedColorSpace: colorSpace, observedEdr: edr };
    } else if (colorSpace !== this.displayColor.observedColorSpace || edr !== this.displayColor.observedEdr) {
      // Legacy GL never reconfigures its drawing buffer for media-query
      // changes. Match that lifetime, including later geometry updates:
      // a first-show capability change must not rebuild a playing pool.
      this.options.log?.('info', `engine presentation display capabilities changed to ${colorSpace}, edr=${edr}; keeping configured ${this.displayColor.colorSpace}, edr=${this.displayColor.edr}`);
      this.displayColor.observedColorSpace = colorSpace;
      this.displayColor.observedEdr = edr;
    }
    return this.displayColor;
  }

  /** A state as sent: with the newest vsync hint, which is not part of the view. */
  private withHint(state: ViewerState): ViewerState {
    return this.vsync ? { ...state, vsync: this.vsync } : state;
  }

  /** The presenter fitted a new refresh grid: re-send the current view with it (same `viewSeq`). */
  private onVsync(hint: VsyncHint): void {
    this.vsync = hint;
    const state = this.sequencer.current;
    if (state) this.presenter?.setViewerState(this.withHint(state));
  }

  /** The size to send: a new size only once it held still (live resizes). */
  private settledSize(view: Window, width: number, height: number): { width: number; height: number } {
    const applied = this.appliedSize;
    const settleMs = this.options.sizeSettleMs ?? SIZE_SETTLE_MS;
    if (!applied || this.settleDue || settleMs <= 0 || (applied.width === width && applied.height === height)) {
      this.settleDue = false;
      if (this.settleTimer) this.clearTimer(view, this.settleTimer);
      this.settleTimer = 0;
      this.pendingSize = null;
      this.appliedSize = { width, height };
      return this.appliedSize;
    }
    // Param/overlay updates repeat during playback. They do not mean the
    // stage resized again: extending this deadline on every repeat can leave
    // the old pool alive for seconds, then reallocate it in steady playback.
    if (this.settleTimer && this.pendingSize?.width === width && this.pendingSize.height === height) return applied;
    if (this.settleTimer) this.clearTimer(view, this.settleTimer);
    this.pendingSize = { width, height };
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

  /** The per-frame trace of the current session since the last call (the S1 measurement harness). */
  takeTrace(): Promise<TraceRecord | null> {
    return this.presenter?.takeTrace() ?? Promise.resolve(null);
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
    const moved = this.host !== null;
    if (this.host) {
      // A popout owns a different timer queue; closing the old host cancels
      // its timer, so a pending size must start a fresh deadline here.
      if (this.settleTimer) this.clearTimer(this.host, this.settleTimer);
      this.settleTimer = 0;
      this.pendingSize = null;
      this.settleDue = false;
    }
    this.host = view;
    for (const off of this.mediaOffs.splice(0)) off();
    const refresh = (): void => {
      if (this.params) this.update(this.params);
    };
    // A popout is sized after it opens: its resizes count as layout.
    view.addEventListener?.('resize', refresh);
    this.mediaOffs.push(() => view.removeEventListener?.('resize', refresh));
    // A live resize delays the panel's presentation: timed frames are
    // picked for the later display while it lasts (`resizeLatencyRefreshes`).
    const resized = (): void => this.presenter?.noteResize?.();
    view.addEventListener?.('resize', resized);
    this.mediaOffs.push(() => view.removeEventListener?.('resize', resized));
    for (const query of ['(dynamic-range: high)', '(color-gamut: p3)', `(resolution: ${view.devicePixelRatio || 1}dppx)`]) {
      const list = view.matchMedia?.(query);
      if (!list?.addEventListener) continue;
      list.addEventListener('change', refresh);
      this.mediaOffs.push(() => list.removeEventListener('change', refresh));
    }
    if (this.worker && moved) {
      // A presentation worker draws into the canvas of the window it was
      // made for: the new window gets a new canvas and worker.
      void this.replaceDrawing();
      return view;
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

  /**
   * Makes the canvas's drawing: a presentation worker when the viewer can
   * start one and the canvas can hand its drawing over, otherwise a WebGL2
   * painter on this thread. False when neither works.
   */
  private makeDrawing(): Promise<boolean> {
    if (this.worker || this.painter) return Promise.resolve(true);
    if (this.drawingPending?.token === this.backendToken) return this.drawingPending.promise;
    const token = this.backendToken;
    const promise = this.createDrawing();
    this.drawingPending = { token, promise };
    const clear = (): void => { if (this.drawingPending?.promise === promise) this.drawingPending = null; };
    void promise.then(clear, clear);
    return promise;
  }

  private async createDrawing(): Promise<boolean> {
    if (this.worker || this.painter) return true;
    const token = this.backendToken;
    // Without `createWorker` (offMainThread false) frames are drawn on the
    // page's main thread: a prepared worker drawing is not claimed.
    const warm = this.options.service.drawingWarmup?.take(this.options.stage.ownerDocument, {
      onFailed: message => this.workerFailed(token, message),
      onLost: message => this.deviceLost(token, message)
    }, this.options.createWorker !== undefined && !this.workerUnavailable);
    if (warm) {
      const old = this.canvasElement;
      this.canvasElement = this.makeCanvas(old.ownerDocument, warm.canvas);
      old.replaceWith(this.canvasElement);
      this.worker = warm.worker;
      if (this.worker) return true;
      const painter = await warm.painter;
      if (this.disposed || token !== this.backendToken) {
        painter?.dispose();
        return false;
      }
      if (painter) { this.painter = painter; return true; }
    }
    const canvas = this.canvasElement;
    const createWorker = this.options.createWorker;
    if (createWorker && !this.workerUnavailable && typeof canvas.transferControlToOffscreen === 'function') {
      try {
        const offscreen = canvas.transferControlToOffscreen();
        const token = this.backendToken;
        this.worker = new PresentWorker({
          worker: createWorker(),
          canvas: offscreen,
          // This realm's clock: arrivals are stamped with it, and a
          // popout's worker must decide on it too (timeOriginShift).
          timeOrigin: performance.timeOrigin,
          onFailed: (message) => this.workerFailed(token, message),
          onLost: (message) => this.deviceLost(token, message)
        });
        return true;
      } catch (error) {
        this.workerUnavailable = true;
        this.options.log?.('warn', `engine presentation stays on the main thread: ${error instanceof Error ? error.message : String(error)}`);
        // The canvas may have handed its drawing over already: use a new one.
        this.replaceCanvas();
      }
    }
    if (this.painterFailed) return false;
    let painter: FramePainter;
    try {
      painter = await (this.options.createPainter ?? createFramePainter)(this.canvasElement);
    } catch (error) {
      this.painterFailed = true;
      const message = error instanceof Error ? error.message : String(error);
      this.options.log?.('error', `engine presentation is unavailable: ${message}`);
      // A visible, named failure, never another engine's frames.
      if (this.exclusive) this.setStatus(`${PRESENTATION_STATUS.unavailable} ${message}`);
      return false;
    }
    if (this.disposed || token !== this.backendToken || this.painter) {
      painter.dispose();
      return !this.disposed && token === this.backendToken;
    }
    this.painter = painter;
    void painter.lost?.then((message) => {
      if (this.painter === painter) this.deviceLost(token, message);
    });
    return true;
  }

  /**
   * The drawing's graphics context was lost (the GPU process restarted): a
   * new canvas and context (a new worker when presenting from one), and the
   * session carries on. Losses that keep coming back off the drawing are
   * given up after `MAX_DEVICE_LOSSES` within `DEVICE_LOSS_WINDOW_MS`, so a
   * broken GPU cannot spin the viewer; the current engine's canvas shows.
   */
  private deviceLost(token: number, message: string): void {
    if (this.disposed || token !== this.backendToken) return;
    const now = performance.now();
    this.losses = this.losses.filter((t) => now - t < DEVICE_LOSS_WINDOW_MS);
    this.losses.push(now);
    if (this.losses.length > MAX_DEVICE_LOSSES) {
      this.options.log?.('error', `engine presentation stopped: the graphics context was lost ${this.losses.length} times in ${DEVICE_LOSS_WINDOW_MS / 1000} s (${message})`);
      this.painterFailed = true;
      this.workerUnavailable = true;
      // A visible failure, never another engine's frames.
      if (this.exclusive) this.setStatus(PRESENTATION_STATUS.gpuFailed);
      void this.replaceDrawing();
      return;
    }
    if (this.exclusive) this.setStatus(PRESENTATION_STATUS.reconnecting);
    this.options.log?.('warn', `engine presentation lost its graphics context (${message}); drawing again on a new one`);
    // The GPU process that held this window's surface imports may be gone
    // with the context: frames of those imports no longer upload, so the
    // new drawing starts on new surfaces.
    void this.replaceDrawing(`the graphics context was lost (${message})`);
  }

  /**
   * The imports of the current pool cannot be read (after a GPU process
   * restart a `VideoFrame` of an earlier import names a shared image the
   * new process does not have): asks the engine for new surfaces with the
   * next surface epoch, which is a new view, so a paused view draws again
   * too. One renewal per pool generation; at most `MAX_SURFACE_RENEWALS`
   * within `DEVICE_LOSS_WINDOW_MS`, after which the stage says so.
   */
  private renewSurfaces(generation: number, why: string): void {
    if (this.disposed || !this.presenter || generation <= this.renewedFrom) return;
    const now = performance.now();
    this.renewals = this.renewals.filter((t) => now - t < DEVICE_LOSS_WINDOW_MS);
    if (this.renewals.length >= MAX_SURFACE_RENEWALS) {
      // Frames of this pool keep failing: once per pool, a visible failure.
      this.renewedFrom = Math.max(this.renewedFrom, generation);
      this.options.log?.('error', `engine presentation cannot read its surfaces: ${this.renewals.length} renewals in ${DEVICE_LOSS_WINDOW_MS / 1000} s (${why})`);
      if (this.exclusive) this.setStatus(PRESENTATION_STATUS.unreadable);
      return;
    }
    const state = this.sequencer.renewSurfaces();
    if (!state) return;
    this.renewals.push(now);
    this.renewedFrom = Math.max(this.renewedFrom, generation, this.presenter.stats.generation);
    this.options.log?.('warn', `engine presentation asks for new surfaces: ${why}`);
    this.states.push(state);
    if (this.states.length > STATE_HISTORY) this.states.shift();
    this.presenter.setViewerState(this.withHint(state));
  }

  private workerFailed(token: number, message: string): void {
    if (this.disposed || token !== this.backendToken) return;
    this.workerUnavailable = true;
    this.options.log?.('warn', `engine presentation worker failed, presenting on the main thread: ${message}`);
    void this.replaceDrawing();
  }

  /** A fresh canvas in place of the current one (in the stage's current document), hidden until it presents. */
  private replaceCanvas(): void {
    const old = this.canvasElement;
    const next = this.makeCanvas(this.options.stage.ownerDocument);
    if (old.parentNode) old.parentNode.replaceChild(next, old);
    else this.options.stage.insertBefore(next, this.options.before && this.options.before.parentElement === this.options.stage ? this.options.before : null);
    this.canvasElement = next;
    this.hide(old);
  }

  /**
   * Replaces the canvas and its drawing (worker moved windows, or failed)
   * and carries the open session over to a new presenter. `renew` (why):
   * the new presenter starts on new surfaces (`renewSurfaces`).
   */
  private async replaceDrawing(renew: string | null = null): Promise<void> {
    const token = ++this.backendToken;
    const generation = this.presenter?.stats.generation ?? 0;
    this.chromeLeases?.clear();
    this.stopPresenter(this.presenter);
    this.presenter = null;
    this.latestFrame = null;
    this.worker?.dispose();
    this.worker = null;
    this.painter?.dispose();
    this.painter = null;
    this.replaceCanvas();
    if (!(await this.makeDrawing()) || token !== this.backendToken || this.disposed) return;
    const channel = this.channel;
    if (!channel) return;
    this.startPresenter(channel, generation);
    if (renew !== null) this.renewSurfaces(Math.max(generation, 1), renew);
  }

  /** A presenter for `channel` on the current drawing, sent the current state. */
  private startPresenter(channel: EngineViewerChannel, generation = 0): void {
    const view = this.options.stage.ownerDocument.defaultView;
    const leases = this.chromeLeases;
    const common = {
      link: {
        sendViewerState: (state: ViewerState) => channel.sendViewerState(state),
        release: (release: import('./protocol').FrameRelease) => leases ? leases.colorDone(release) : channel.release(release),
      },
      source: {
        videoFrame: (generation: number, index: number) => this.options.service.source.videoFrame(generation, index),
        onSurface: (listener: (generation: number, index: number) => void) => this.options.service.source.onSurface(listener),
        retire: (generation: number) => leases ? leases.retire(generation, () => this.options.service.source.retire(generation)) : this.options.service.source.retire(generation),
      },
      trace: this.options.trace,
      generation,
      hostOffsetUs: this.options.service.hostOffsetUs ? () => this.options.service.hostOffsetUs!() : undefined,
      onVsync: (hint: VsyncHint) => this.onVsync(hint),
      onUnusable: (generation: number, message: string) => {
        if (this.presenter === presenter) this.renewSurfaces(generation, message);
      }
    };
    const presenter: Presenter = this.worker
      ? this.worker.open({ ...common, displayHz: view ? (this.options.service.displayHz?.(view) ?? null) : null })
      : new EnginePresenter({ ...common, painter: this.painter! });
    this.presenter = presenter;
    presenter.onPresented((frame) => {
      if (this.presenter !== presenter) return;
      // A frame drawn before the engine routed the composition to the
      // current engine, presented late: it must not cover the current
      // engine's canvas.
      if (this.routedSeq !== null && frame.seq <= this.routedSeq) return;
      this.routedSeq = null;
      leases?.drawing(frame.seq);
      // A canvas that was hidden shows a frame only once the GPU finished
      // it and the display took it (the drawing's canvas and this page
      // commit separately; at a cold start the drawing's pipelines compile
      // first): it shows at once, so the page has committed it by then, and
      // `latest`, `presenting`, and the frame events wait for the frame
      // (`reveal`). A new presenter (a restart, a replaced drawing) starts
      // its own wait.
      if (this.canvasElement.style.visibility !== 'visible') {
        this.canvasElement.style.visibility = 'visible';
        this.revealing = frame;
        this.reveal(presenter);
        return;
      }
      if (this.revealing) {
        this.revealing = frame;
        if (this.revealFor !== presenter) this.reveal(presenter);
        return;
      }
      if (this.statusElement) this.setStatus(null);
      this.show(frame);
    });
    this.host = null;
    if (!this.follow()) presenter.start();
    // Messages that came while there was no presenter (a replaced drawing).
    for (const message of this.held.splice(0)) presenter.receive(message.message, message.arrivedMs);
    // A new presenter knows nothing yet: send the current state.
    const state = this.sequencer.current;
    if (state) presenter.setViewerState(this.withHint(state));
    else if (this.params) this.update(this.params);
  }

  private async open(): Promise<void> {
    if (this.disposed || this.channel) return;
    const token = this.backendToken;
    if (!(await this.makeDrawing())) return;
    if (this.disposed || this.channel || token !== this.backendToken) return;
    let opened: EngineViewerChannel | null = null;
    const channel = this.options.service.open({
      message: (message, arrivedMs) => {
        if (this.channel !== opened) return;
        if (message.type === 'surfacePool') {
          let leases = this.poolLeases.get(message.pool.generation);
          if (!leases) { leases = new Set(); this.poolLeases.set(message.pool.generation, leases); }
          leases.add(this.chromeLeases);
        }
        if (message.type === 'routed') {
          // The engine cannot draw this composition at this revision: its
          // frames (older ones still in flight too) leave the stage.
          this.routedSeq = Math.max(this.deliveredSeq, Number(message.seq) || 0);
          this.latestFrame = null;
          this.chromeLeases?.clear();
          if (this.exclusive) {
            // "Use New Engine" on (amendment 21): nothing else draws it, so
            // the stage says why, and stays the engine's (no hand-back; the
            // transport keeps running on the engine's clock). The next
            // frame the engine draws clears it.
            this.setStatus(routedStatus(message.reason));
            this.hide(this.canvasElement, false);
            return;
          }
          this.setStatus(null);
          this.hide(this.canvasElement);
          return;
        }
        if (message.type === 'frame') {
          this.deliveredSeq = Math.max(this.deliveredSeq, Number(message.frame.seq) || 0);
          this.chromeLeases?.track(message.frame);
        }
        if (this.presenter) this.presenter.receive(message, arrivedMs);
        else this.hold(opened!, message, arrivedMs);
      },
      closed: (reason) => this.closed(opened, reason)
    });
    if (!channel) return;
    opened = channel;
    this.channel = channel;
    this.chromeLeases = new ChromeLeases(release => channel.release(release));
    // A new session numbers its frames from 1, and a restarted engine its
    // pool generations.
    this.deliveredSeq = 0;
    this.renewedFrom = 0;
    this.routedSeq = null;
    this.sessions++;
    this.startPresenter(channel);
  }

  /**
   * A message while the drawing is being replaced: a frame goes straight
   * back to the engine (nothing will draw it), pools and clock samples wait
   * for the next presenter.
   */
  private hold(channel: EngineViewerChannel, message: EngineFrameMessage, arrivedMs: number): void {
    if (message.type === 'frame') {
      const release = { poolIndex: message.frame.poolIndex, seq: message.frame.seq };
      if (this.chromeLeases) this.chromeLeases.colorDone(release);
      else channel.release(release);
    }
    else this.held.push({ message, arrivedMs });
  }

  private closed(channel: EngineViewerChannel | null, reason: string): void {
    if (!channel || channel !== this.channel) return;
    this.options.log?.('info', `engine viewer session ended: ${reason}`);
    this.channel = null;
    this.chromeLeases?.clear();
    this.chromeLeases = null;
    this.held = [];
    this.stopPresenter(this.presenter);
    this.presenter = null;
    if (this.exclusive) {
      // The engine restarts (section 19): the last frame stays on screen,
      // with its tags for the overlay, until the new session presents.
      this.setStatus(PRESENTATION_STATUS.reconnecting);
      return;
    }
    this.latestFrame = null;
    this.hide(this.canvasElement);
  }

  /** The output scale a frame was drawn at (today's preview quality factor). */
  private qualityOf(frame: PresentedFrame): number {
    const state = this.states.find((s) => Number(s.viewSeq) === frame.viewSeq) ?? this.sequencer.current;
    switch (state?.resolution) {
      case 'half': return 0.5;
      case 'quarter': return 0.25;
      default: return 1;
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    const leases = this.chromeLeases;
    leases?.clear();
    this.chromeLeases = null;
    this.backendToken++;
    if (this.settleTimer && this.host) this.clearTimer(this.host, this.settleTimer);
    this.settleTimer = 0;
    this.offReady();
    for (const off of this.mediaOffs.splice(0)) off();
    this.stopPresenter(this.presenter);
    this.presenter = null;
    const channel = this.channel;
    channel?.close();
    this.channel = null;
    this.worker?.dispose();
    this.worker = null;
    this.painter?.dispose();
    this.painter = null;
    const pools = [...this.poolLeases];
    this.poolLeases.clear();
    // The preload's imports are otherwise retained until a replacement
    // pool or pagehide. A worker acknowledges `closed` only after its GPU
    // work and page-side VideoFrame references have drained; chrome reads
    // can defer retirement further through their leases.
    if (channel?.retireClosedPool) void Promise.all(this.draining).then(() => {
      for (const [generation, holders] of pools) {
        let remaining = holders.size;
        const oneDrained = (): number => {
          if (--remaining === 0) return channel.retireClosedPool?.(generation) ?? 0;
          return 0;
        };
        for (const poolLeases of holders) {
          if (poolLeases) poolLeases.retire(generation, oneDrained);
          else oneDrained();
        }
      }
    });
    this.listeners.clear();
    this.hiddenListeners.clear();
    this.canvasElement.remove();
    this.setStatus(null);
    if (this.options.stage.dataset['engineStage'] === (this.options.role ?? 'viewer')) delete this.options.stage.dataset['engineStage'];
  }

  private stopPresenter(presenter: Presenter | null): void {
    if (!presenter) return;
    presenter.dispose();
    const drained = presenter.drained?.();
    if (!drained) return;
    this.draining.add(drained);
    void drained.then(() => this.draining.delete(drained), () => this.draining.delete(drained));
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
