/*
 * Frame events and stats while the engine presents (the Phase 3c render
 * transport, plan section 20 "events": frame events on presentation;
 * `frame:rendered.viewport` identity kept), behind "Use New Engine".
 *
 * Today the compositor emits `preview:presented` from inside `GL.render`
 * (the kernel forwards it as `frame:rendered` with the exact
 * `render.gl.previewViewport` object) and the frame loop emits `overlay`
 * after it. While the engine presents, the compositor draws nothing, so
 * the engine connection emits the same two, in the same order, once per
 * new frame the viewer puts on screen:
 *
 * - `time`: the frame's presentation time in composition seconds;
 * - `viewport`: the object `render.gl.previewViewport` returns at that
 *   moment (identity, not a copy);
 * - `version`: `PM.animVersion()` as it was when the frame's revision was
 *   sent to the engine (not the newest, which may lead the pixels);
 * - `quality`: the output scale the frame was rendered at.
 *
 * Engine stats (`RenderStats`, at most twice a second per session) replace
 * `GL.stats` and GPU timing for the status bar and the performance panel:
 * `EngineStatsView` turns them into today's `perf` fields.
 */
import type { RenderStats } from '../../../shared/engine-protocol';
import { TICKS_PER_SECOND } from './transport';

/** `PM.animVersion()` per document revision, as each revision was sent. */
export class RevisionVersions {
  private readonly entries: Array<[number, number | undefined]> = [];

  constructor(private readonly cap = 512) {}

  /** Records the version current when `revision` left for the engine. */
  note(revision: number, version: number | undefined): void {
    const last = this.entries.at(-1);
    if (last && last[0] === revision) {
      last[1] = version;
      return;
    }
    if (last && revision < last[0]) this.entries.length = 0;
    this.entries.push([revision, version]);
    if (this.entries.length > this.cap) this.entries.shift();
  }

  /** The version of the newest revision at or below `revision`. */
  at(revision: number): number | undefined {
    for (let i = this.entries.length - 1; i >= 0; i--) if (this.entries[i]![0] <= revision) return this.entries[i]![1];
    return undefined;
  }
}

/** A presented frame, as the viewer's presentation reports it. */
export type PresentedFrameLike = { pts: number; revision: number; viewSeq: number; seq: number };

export type FrameRenderedPayload = { viewport: unknown; time: number; version: number | undefined; project: unknown; quality: number };

/** Emits today's `preview:presented` (→ `frame:rendered`) and `overlay` for a presented engine frame. */
export function emitPresented(
  bus: { emit(event: string, payload?: unknown): void } | null | undefined,
  frame: PresentedFrameLike,
  quality: number,
  context: { viewport: () => unknown; versions: RevisionVersions; project: () => unknown }
): FrameRenderedPayload {
  const payload: FrameRenderedPayload = {
    viewport: context.viewport(),
    time: frame.pts / TICKS_PER_SECOND,
    version: context.versions.at(frame.revision),
    project: context.project(),
    quality
  };
  bus?.emit('preview:presented', payload);
  bus?.emit('overlay');
  return payload;
}

/**
 * Coalesces presented frames to one emission per animation frame (the
 * newest frame wins), as today's frame loop emits `preview:presented` and
 * `overlay` once per frame it draws, and the overlay redraws in the
 * animation frame rather than inside the presentation's callback.
 * `listenersMs` and `slowestMs` record what the listeners cost on the
 * page's main thread (measured 2026-10-18 during 30 fps playback: 88
 * emissions in 3 s took 2.2 ms in all, the slowest 0.1 ms).
 */
export class PresentedEvents {
  private pending: { frame: PresentedFrameLike; quality: number } | null = null;
  private scheduled = false;
  emitted = 0;
  /** Main-thread time the emissions' listeners took (ms), and the slowest. */
  listenersMs = 0;
  slowestMs = 0;

  constructor(
    private readonly emit: (frame: PresentedFrameLike, quality: number) => void,
    private readonly nextFrame: (fn: () => void) => void
  ) {}

  push(frame: PresentedFrameLike, quality: number): void {
    this.pending = { frame, quality };
    if (this.scheduled) return;
    this.scheduled = true;
    this.nextFrame(() => {
      this.scheduled = false;
      const next = this.pending;
      this.pending = null;
      if (!next) return;
      this.emitted++;
      const started = performance.now();
      this.emit(next.frame, next.quality);
      const ms = performance.now() - started;
      this.listenersMs += ms;
      this.slowestMs = Math.max(this.slowestMs, ms);
    });
  }
}

/** Today's `perf` fields (runtime/install-legacy.ts) from engine stats. */
export type PerfFields = { ms: number; fps: number; draws: number; passes: number; progs: number; raster: number };

/** Engine stats for the status bar and the performance panel. */
export class EngineStatsView {
  latest: RenderStats | null = null;
  private window: { at: number; frames: number } | null = null;
  private fps = 0;

  /** A stats message arrived at `nowMs` (performance.now()). */
  receive(stats: RenderStats, nowMs: number): void {
    const frames = Number(stats.frames);
    if (this.window && frames >= this.window.frames && nowMs > this.window.at) {
      const dt = nowMs - this.window.at;
      const df = frames - this.window.frames;
      // Presented frames per second over the stats interval (today's E.fps
      // counts frames drawn every 500 ms the same way).
      this.fps = df > 0 ? Math.round((df * 1000) / dt) : 0;
    }
    this.window = { at: nowMs, frames };
    this.latest = stats;
  }

  /** The fields, or null before the first stats message. */
  perf(): PerfFields | null {
    const s = this.latest;
    if (!s) return null;
    return {
      ms: s.frameUsAvg / 1000,
      fps: this.fps,
      draws: s.layers,
      passes: 0,
      progs: s.programs,
      raster: s.rasters
    };
  }
}
