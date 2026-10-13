/*
 * The page half of the real-viewer measurement (amendment 18: before the
 * Phase 2 gate, "today" is the real WebGL viewer, not the harness's
 * stand-in). measure-app-main.ts injects this bundle into the real app's
 * editor window after it opened a corpus project; scripts/
 * frame-bridge-measure.mjs analyzes what `collect()` returns with the same
 * code as the stand-in runs.
 *
 * Two modes, both in the real app with the real project, panels, timeline,
 * and overlay:
 *
 * - `today`: the WebGL viewer draws the project. Every compositor draw
 *   (`preview:presented`, emitted synchronously inside `PM.GL.render`) is
 *   recorded with the timestamp of the animation frame it ran in, the
 *   content time it drew, and the playback line's offset, so each content
 *   frame's due time is known exactly (the legacy clock is
 *   `PM.time = base + (rAF − origin) / 1000`).
 * - `engine`: the page plays the kernel's part (the engine connection is
 *   not in the kernel yet): it registers `engine.viewer` with the bridge's
 *   page engine connection, so the real viewer attaches its
 *   `ViewerPresentation` over the stage and presents the interactive
 *   engine's frames. The WebGL compositor's draw is turned off (with the
 *   new engine on it would not draw), and the legacy transport still plays,
 *   so the timeline, playhead, and panels do their per-frame work as they
 *   would with the engine's clock.
 *
 * Gestures. `drag` (today) presses on a layer with the Select tool and
 * moves it; each draw is tagged with the newest handled pointer event whose
 * layer position it shows. `pan` presses the middle button on the stage
 * and pans the view: the same viewer handler on both paths. Today moves
 * the WebGL canvas with CSS, so every animation frame is tagged with the
 * newest handled event; the engine renders the panned view, and its frames
 * echo the view sequence. Pointer coordinates encode their sequence
 * numbers.
 */
import { ENGINE_VIEWER_CHANGED, ENGINE_VIEWER_SERVICE, type EngineViewerService } from '../../../extensions/viewer/engine-present/presentation';
import type { TransportCommand } from '../../../shared/engine-protocol';
import type { ClockSample } from '../../../extensions/viewer/engine-present/protocol';
import { PageEngine } from './page-engine';

/* eslint-disable @typescript-eslint/no-explicit-any */
type Mode = 'today' | 'engine';
type InputMode = 'drag' | 'pan';
type InitOptions = { mode: Mode; displayHz: number | null; rawGestures: boolean };

const w = window as any;
const PM = (): any => w.PM;

/* ── animation frames and compositor draws ─────────────────────────── */

const rafs: number[] = [];
let inRaf: number | null = null;
/** Draws made outside an animation frame show at the next one. */
let pendingDraws: ((rafMs: number) => void)[] = [];
const nativeRaf = window.requestAnimationFrame.bind(window);
window.requestAnimationFrame = (callback: FrameRequestCallback): number =>
  nativeRaf((t) => {
    if (rafs[rafs.length - 1] !== t) {
      rafs.push(t);
      const pending = pendingDraws;
      pendingDraws = [];
      for (const assign of pending) assign(t);
    }
    inRaf = t;
    try {
      callback(t);
    } finally {
      inRaf = null;
    }
  });

/** Today's draws: [rafMs, frame index, content ticks (unused), rev, PM.time s, line offset ms, quality]. */
let draws: [number, number, number, number, number, number, number][] = [];
let fps = 30;
let mode: Mode = 'today';
let engine: PageEngine | null = null;
const clocks: ClockSample[] = [];

/* ── input ─────────────────────────────────────────────────────────── */

let inputMode: InputMode | null = null;
let origin = { x: 0, y: 0 };
let lastSeq = -1;
let dragLayer: any = null;
/** Per dispatch: [rev, seqs, event timeStamp ms, handled ms]. */
let inputs: [number, number[], number, number][] = [];
/** Today's drag: handled seq → layer position key. */
let positions: [number, string][] = [];
/** Today's pan: the newest handled pointer sequence. */
let panSeq = -1;
let panLoop = false;

/**
 * Today's pan: every animation frame commits the canvas position the
 * handled pointer events set (rAF-aligned pointer events are dispatched
 * before the frame's callbacks), so each frame is a draw tagged with the
 * newest handled sequence.
 */
function panFrames(): void {
  if (panLoop) return;
  panLoop = true;
  const tick = (t: number): void => {
    if (inputMode !== 'pan' || mode !== 'today') {
      panLoop = false;
      return;
    }
    if (panSeq >= 0) draws.push([t, 0, 0, panSeq, 0, 0, 1]);
    nativeRaf(tick);
  };
  nativeRaf(tick);
}

/** Durations of the viewer's layout calls while an input mode runs (ms). */
let layoutMs: number[] = [];
const CODE_X = 400;
const CODE_SPAN = CODE_X * 6;

/** Undoes measure-app-main.ts's coordinate code (x = x0 − 200 + seq mod 400,
 *  y = y0 + ⌊seq / 400⌋ mod 6) and unwraps it past the last sequence. */
function decode(clientX: number, clientY: number): number {
  const code = Math.round(clientX - origin.x + 200) + Math.round(clientY - origin.y) * CODE_X;
  if (lastSeq < 0) return code;
  let seq = code + Math.floor((lastSeq - code) / CODE_SPAN) * CODE_SPAN;
  while (seq < lastSeq) seq += CODE_SPAN;
  if (seq - lastSeq > CODE_SPAN / 2) seq -= CODE_SPAN;
  return seq;
}

function positionKey(): string {
  const pm = PM();
  if (!dragLayer) return '';
  const layer = pm.proj.layers.find((l: any) => l.id === dragLayer.id) ?? dragLayer;
  const t = pm.time;
  return `${pm.ev(layer, 'position.x', t)},${pm.ev(layer, 'position.y', t)}`;
}

function viewSeq(): number {
  return viewer()?.enginePresentation?.state?.viewSeq ?? 0;
}

const viewer = (): any => PM()?.Kernel?.services?.get('viewer') ?? null;

function onPointer(event: PointerEvent): void {
  if (!inputMode) return;
  const handled = performance.now();
  const events = event.getCoalescedEvents?.().length ? event.getCoalescedEvents() : [event];
  const seqs = events.map((e) => {
    const seq = decode(e.clientX, e.clientY);
    lastSeq = Math.max(lastSeq, seq);
    return seq;
  });
  const seq = seqs[seqs.length - 1]!;
  if (inputMode === 'drag') {
    positions.push([seq, positionKey()]);
    inputs.push([seq, seqs, event.timeStamp, handled]);
  } else if (mode === 'today') {
    // Today's pan moves the WebGL canvas with CSS: the next animation
    // frame's commit shows it (panFrames).
    panSeq = seq;
    inputs.push([seq, seqs, event.timeStamp, handled]);
  } else {
    inputs.push([viewSeq(), seqs, event.timeStamp, handled]);
  }
}
// Bubble phase on the window: after the viewer's own (capture-phase) drag
// handlers applied the event.
window.addEventListener('pointermove', (event) => onPointer(event));
window.addEventListener('pointerrawupdate' as 'pointermove', (event) => {
  // The viewer reads raw updates during engine gestures (step 2); tag them
  // the same way so the view sequence they produce is attributed.
  if (inputMode === 'pan' && viewer()?.rawGestures?.()) onPointer(event as PointerEvent);
});

/** The rev a today draw shows: the newest handled seq whose position it has. */
function drawnRev(): number {
  const key = positionKey();
  for (let i = positions.length - 1; i >= 0; i--) if (positions[i]![1] === key) return positions[i]![0];
  return -1;
}

function onPresented(frame: { time: number }): void {
  const pm = PM();
  const t = Number(frame?.time ?? pm.time);
  const now = Number(pm.time);
  const rev = inputMode === 'drag' ? drawnRev() : -1;
  const record = (rafMs: number): void => {
    draws.push([rafMs, Math.round(t * fps), 0, rev, now, rafMs - now * 1000, Number(pm.quality ?? 1)]);
  };
  if (inRaf !== null) record(inRaf);
  else pendingDraws.push(record);
}

/* ── the engine path in the real viewer ───────────────────────────── */

function startEngine(displayHz: number | null): void {
  engine = new PageEngine(displayHz);
  const source = engine;
  const service: EngineViewerService & { trace: boolean } = {
    source: source.source,
    trace: true,
    composition: () => source.composition(),
    displayHz: () => source.displayHz(),
    onReady: (listener) => source.onReady(listener),
    open: (handlers) =>
      source.open({
        message: (message, arrivedMs) => {
          if (message.type === 'clock') clocks.push(message.sample);
          handlers.message(message, arrivedMs);
        },
        closed: handlers.closed
      })
  };
  PM().Kernel.services.register(ENGINE_VIEWER_SERVICE, service);
  window.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
  // With the new engine on, the WebGL compositor does not draw the viewer.
  const gl = PM().GL;
  gl.render = () => true;
}

/* ── the API measure-app-main.ts calls ────────────────────────────── */

const api = {
  init(options: InitOptions): void {
    mode = options.mode;
    fps = Number(PM().proj.fps) || 30;
    PM().bus.on('preview:presented', onPresented);
    if (mode === 'engine') startEngine(options.displayHz);
    // Before step 2: the viewer's drags keep pointermove.
    if (!options.rawGestures) viewer().rawGestures = () => false;
    // How long the viewer's layout (pan steps) holds the main thread.
    const v = viewer();
    const layout = v.layout.bind(v);
    v.layout = (...args: unknown[]): unknown => {
      if (!inputMode) return layout(...args);
      const t0 = performance.now();
      try {
        return layout(...args);
      } finally {
        layoutMs.push(+(performance.now() - t0).toFixed(3));
      }
    };
  },
  ready(): boolean {
    if (mode === 'engine') return !!viewer()?.enginePresentation?.presenting;
    return draws.length > 0 || rafs.length > 30;
  },
  /** Plays from `fromS`: the legacy transport, and the engine's when on. */
  play(fromS: number): boolean {
    const pm = PM();
    pm.setTime(fromS);
    pm.play();
    if (mode === 'engine') {
      const channel = engine?.channels.at(-1);
      if (!channel || channel.session() === null) return false;
      channel.transport({ command: 'play', rate: { num: 1, den: 1 } });
    }
    return true;
  },
  pause(): void {
    PM().pause();
    if (mode === 'engine') engine?.channels.at(-1)?.transport({ command: 'pause' });
  },
  transport(command: TransportCommand): boolean {
    const channel = engine?.channels.at(-1);
    if (!channel || channel.session() === null) return false;
    channel.transport(command);
    return true;
  },
  mark(name: string): number {
    performance.mark(name);
    return performance.now();
  },
  now: (): number => performance.now(),
  /** The viewer stage's top left in the window's content (CSS pixels = points). */
  stageRect(): { left: number; top: number } | null {
    const r = viewer()?.stage?.getBoundingClientRect();
    return r ? { left: r.left, top: r.top } : null;
  },
  reset(): void {
    viewer()?.enginePresentation?.takeTrace();
    draws = [];
    rafs.length = 0;
    inputs = [];
    positions = [];
    layoutMs = [];
    clocks.length = 0;
  },
  /**
   * Prepares a gesture and returns where to press (client pixels) and with
   * which button. `drag`: the first unlocked, visible, non-group layer under
   * a grid of stage points, selected. `pan`: the stage center.
   */
  prepare(m: InputMode): { x: number; y: number; button: 'left' | 'middle' } | { error: string } {
    const v = viewer();
    const stage: HTMLElement | null = v?.stage ?? null;
    if (!stage) return { error: 'no viewer stage' };
    const rect = stage.getBoundingClientRect();
    lastSeq = -1;
    if (m === 'pan') {
      origin = { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
      return { ...origin, button: 'middle' };
    }
    for (let gy = 0.3; gy <= 0.7; gy += 0.05) {
      for (let gx = 0.3; gx <= 0.7; gx += 0.05) {
        const x = Math.round(rect.left + rect.width * gx);
        const y = Math.round(rect.top + rect.height * gy);
        const layer = v.layerAtPoint?.(x, y);
        if (!layer || layer.lock || layer.type === 'group' || layer.type === 'null') continue;
        dragLayer = layer;
        origin = { x, y };
        return { x, y, button: 'left' };
      }
    }
    return { error: 'no draggable layer under the stage' };
  },
  setInputMode(m: InputMode | null): void {
    inputMode = m;
    panSeq = -1;
    if (m === 'pan' && mode === 'today') panFrames();
  },
  collect(): unknown {
    const pm = PM();
    const presentation = viewer()?.enginePresentation ?? null;
    const trace = presentation?.takeTrace?.() ?? null;
    return {
      mode,
      real: true,
      fps,
      project: { name: pm.proj.name, w: pm.proj.w, h: pm.proj.h, fps: pm.proj.fps, layers: pm.proj.layers.length },
      quality: pm.quality,
      presenter: presentation?.stats?.() ?? null,
      engineTrace: trace,
      today: { draws, rafs: [...rafs] },
      clocks: clocks.splice(0),
      inputs,
      layoutMs,
      dragLayer: dragLayer ? { id: dragLayer.id, type: dragLayer.type } : null,
      state: presentation?.state ?? null,
      receiver: engine?.source.stats() ?? null,
      rawGestures: !!viewer()?.rawGestures?.(),
      stage: (() => {
        const r = viewer()?.stage?.getBoundingClientRect();
        return r ? { width: r.width, height: r.height, dpr: devicePixelRatio } : null;
      })()
    };
  },
  /** Agent tool traffic (the S1 load): a layer listing for a request. */
  agent(request: { id: number }): { id: number; layers: unknown[] } {
    const layers = [];
    for (let i = 0; i < 600; i++) layers.push({ id: `layer-${i}`, name: `Layer ${i}`, in: i * 0.1, out: i * 0.1 + 5, props: { x: i, y: i * 2, opacity: 1, effects: ['blur', 'glow'] } });
    return { id: request.id, layers };
  }
};

w.__measureApp = api;
