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
 * - `engine`: the kernel's engine connection (Use New Engine on) with the
 *   interactive engine drawing its test source at the project's rate
 *   (`POWERMOVE_ENGINE_FRAME_SOURCE=test-pattern`; the 884-layer corpus
 *   project routes to the current engine, so the engine-side render cost
 *   is the test source's). The page wraps the kernel's `engine.viewer` to
 *   ask for the per-frame trace and the presentation thread, and shares the
 *   kernel's connection: the window's interactive port allows one
 *   handshake, so a second connection of the harness's own was refused
 *   (phase2-gate.md, H2). The real viewer attaches its `ViewerPresentation`
 *   over the stage, and the transport runs on the engine's clock, so the
 *   timeline, playhead, and panels do their per-frame work as they do with
 *   the engine presenting.
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
import type { ClockSample } from '../../../extensions/viewer/engine-present/protocol';
import { buildSyntheticProject, type SyntheticOptions } from './synthetic-project';

/* eslint-disable @typescript-eslint/no-explicit-any */
/**
 * `tracer`: the app's own engine connection (the Phase 2 tracer bullet,
 * "Use New Engine" on): the engine draws the document, the transport runs
 * on its clock, and gestures edit the document; frames are tagged with
 * the document revision they show.
 */
type Mode = 'today' | 'engine' | 'tracer';
type InputMode = 'drag' | 'pan';
type InitOptions = { mode: Mode; displayHz: number | null; rawGestures: boolean; offMainThread: boolean };

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
const clocks: ClockSample[] = [];
/** Inspector edits during playback (tracer): [revision, call ms, returned ms]. */
let edits: [number, number, number][] = [];
let editSeq = 0;

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
  if (inputMode === 'drag' && mode === 'tracer') {
    // The drag's edit is in the document revision the barrier holds after
    // this event (the connection flushes at the end of each task; a raw
    // update the viewer holds for its move interval shows in the next).
    const doc = PM().EngineDoc;
    doc?.barrier?.flush();
    const waiting = event.type === 'pointerrawupdate' && viewer()?.rawGesturePending?.() === true;
    inputs.push([(doc?.barrier?.revision ?? 0) + (waiting ? 1 : 0), seqs, event.timeStamp, handled]);
  } else if (inputMode === 'drag') {
    positions.push([seq, positionKey()]);
    inputs.push([seq, seqs, event.timeStamp, handled]);
  } else if (mode === 'today') {
    // Today's pan moves the WebGL canvas with CSS: the next animation
    // frame's commit shows it (panFrames).
    panSeq = seq;
    inputs.push([seq, seqs, event.timeStamp, handled]);
  } else {
    // A raw update the viewer holds for its move interval (raw-gestures.ts)
    // shows in the next view, not the current one.
    const waiting = event.type === 'pointerrawupdate' && viewer()?.rawGesturePending?.() === true;
    inputs.push([viewSeq() + (waiting ? 1 : 0), seqs, event.timeStamp, handled]);
  }
}
// Bubble phase on the window: after the viewer's own (capture-phase) drag
// handlers applied the event.
window.addEventListener('pointermove', (event) => onPointer(event));
window.addEventListener('pointerrawupdate' as 'pointermove', (event) => {
  // The viewer reads raw updates during engine gestures (step 2); tag them
  // the same way so the view sequence they produce is attributed.
  if ((inputMode === 'pan' || (inputMode === 'drag' && mode === 'tracer')) && viewer()?.rawGestures?.()) onPointer(event as PointerEvent);
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

/**
 * The kernel's `engine.viewer`, asking for the per-frame trace and the
 * presentation thread the run measures. Every session opens on the
 * kernel's connection (`open` delegates), so the kernel's transport follows
 * it and the window keeps one handshake on its interactive port.
 */
function startEngine(displayHz: number | null, offMainThread: boolean, testSource: boolean): void {
  const connection = PM().EngineConnection;
  if (!connection) throw new Error('the engine connection is not installed (POWERMOVE_NEW_ENGINE=1)');
  const kernel = PM().Kernel.services.get(ENGINE_VIEWER_SERVICE) as EngineViewerService | null;
  if (!kernel) throw new Error('the kernel did not register engine.viewer');
  connection.onClock((sample: ClockSample) => clocks.push(sample));
  const service: EngineViewerService = {
    source: kernel.source,
    trace: true,
    offMainThread,
    composition: () => kernel.composition(),
    displayHz: (view) => kernel.displayHz?.(view) ?? displayHz,
    onReady: (listener) => kernel.onReady(listener),
    open: (handlers) => kernel.open(handlers),
    ...(kernel.hostOffsetUs ? { hostOffsetUs: () => kernel.hostOffsetUs!() } : {}),
    ...(kernel.revision ? { revision: () => kernel.revision!() } : {})
  };
  PM().Kernel.services.register(ENGINE_VIEWER_SERVICE, service);
  window.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
  // The engine draws its test source over a project it does not route:
  // the WebGL compositor would draw underneath, which it does not with the
  // engine presenting.
  if (testSource) {
    const gl = PM().GL;
    gl.render = () => true;
  }
}

/* ── the API measure-app-main.ts calls ────────────────────────────── */

const api = {
  init(options: InitOptions): void {
    mode = options.mode;
    fps = Number(PM().proj.fps) || 30;
    PM().bus.on('preview:presented', onPresented);
    // `tracer`: the engine draws the document; `engine`: its test source.
    if (mode === 'engine' || mode === 'tracer') startEngine(options.displayHz, options.offMainThread, mode === 'engine');
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
  /** Builds a synthetic project of slice features (synthetic-project.ts). */
  async synthetic(options: SyntheticOptions): Promise<unknown> {
    return buildSyntheticProject(options);
  },
  ready(): boolean {
    if (mode === 'engine') return !!viewer()?.enginePresentation?.presenting && PM().EngineConnection.status().route === 'engine';
    if (mode === 'tracer') return !!viewer()?.enginePresentation?.presenting && PM().EngineConnection.status().route === 'engine';
    return draws.length > 0 || rafs.length > 30;
  },
  /** Plays from `fromS`: the legacy transport, and the engine's when on. */
  play(fromS: number): boolean {
    const pm = PM();
    pm.setTime(fromS);
    pm.play();
    // Both engine modes play through the kernel's transport driver.
    if (mode === 'tracer' || mode === 'engine') return PM().EngineConnection.status().driving === true;
    return true;
  },
  pause(): void {
    PM().pause();
  },
  /**
   * One inspector edit (tracer): the Edit command the Properties panel
   * applies, on a layer of the open composition. Records the revision it
   * made and how long the renderer took to apply it and hand the batch to
   * the engine (the connection flushes at the end of the task).
   */
  async edit(): Promise<void> {
    const pm = PM();
    const layers = pm.proj.layers.filter((l: any) => l.type === 'solid' || l.type === 'image');
    if (!layers.length) return;
    const layer = layers[editSeq++ % layers.length];
    const t0 = performance.now();
    pm.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 30 + (editSeq % 70) }, { label: 'Opacity', origin: 'inspector' });
    await Promise.resolve();
    const t1 = performance.now();
    edits.push([pm.EngineDoc?.barrier?.revision ?? 0, t0, t1]);
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
  async reset(): Promise<void> {
    await viewer()?.enginePresentation?.takeTrace();
    draws = [];
    rafs.length = 0;
    inputs = [];
    positions = [];
    layoutMs = [];
    clocks.length = 0;
    edits = [];
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
  async collect(): Promise<unknown> {
    const pm = PM();
    const presentation = viewer()?.enginePresentation ?? null;
    const trace = (await presentation?.takeTrace?.()) ?? null;
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
      edits,
      inputs,
      layoutMs,
      dragLayer: dragLayer ? { id: dragLayer.id, type: dragLayer.type } : null,
      state: presentation?.state ?? null,
      receiver: (PM().Kernel.services.get(ENGINE_VIEWER_SERVICE)?.source as { stats?: () => unknown } | undefined)?.stats?.() ?? null,
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
