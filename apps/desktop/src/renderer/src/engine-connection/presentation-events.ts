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
 * `EngineStatsView` turns them into today's `perf` fields, and carries the
 * session's audio health (audio-diagnostics.ts).
 */
import type { RenderStats } from '../../../shared/engine-protocol';
import { audioHealth, type AudioHealth } from './audio-diagnostics';
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

/** A presented frame, as the viewer's presentation reports it: a timed
 *  (playback) frame carries the animation frame (`rafMs`, the page's clock)
 *  that drew it. */
export type PresentedFrameLike = { pts: number; revision: number; viewSeq: number; seq: number; timed?: boolean; rafMs?: number };

/**
 * The page's and the presentation worker's animation frames of one refresh
 * carry the same BeginFrame time (within 0.1 ms, measured); a frame drawn
 * more than this before an animation frame was drawn at an earlier refresh.
 */
const SAME_REFRESH_MS = 1;

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
 * Emits presented frames at most once per animation frame, as today's
 * frame loop emits `preview:presented` and `overlay` once per frame it
 * draws, and the overlay redraws in the animation frame rather than inside
 * the presentation's callback.
 *
 * A timed frame is emitted at the first animation frame of a later refresh
 * than the one that drew it, whenever its report reached this thread. The
 * worker draws in its animation frame and its report arrives about a
 * millisecond into the refresh, the moment the page's own animation frame
 * runs; taken in arrival order, a report on either side of that frame
 * left one animation frame with nothing and the next with two, and the
 * older was never announced (docs/engine/bridge.md, "Pacing at 120 Hz").
 * A frame drawn on arrival (paused, scrub, gesture) is on screen when its
 * report arrives: it goes then, as today's frame loop announces the frame
 * it has just drawn. Waiting for the page's next animation frame added a
 * refresh, more while an extension's animation frames keep this thread
 * busy. Another such frame before an animation frame with nothing to
 * announce has passed waits for the next animation frame, so at most one
 * goes per refresh. When the page skipped refreshes, the newest frame due
 * wins (`coalesced` counts the others), as today's loop draws once.
 *
 * `listenersMs` and `slowestMs` record what the listeners cost on the
 * page's main thread (measured 2026-10-18 during 30 fps playback: 88
 * emissions in 3 s took 2.2 ms in all, the slowest 0.1 ms).
 */
export class PresentedEvents {
  /** Reported frames not yet emitted, oldest first. */
  private queue: { frame: PresentedFrameLike; quality: number }[] = [];
  private scheduled = false;
  /** A frame drawn on arrival may go at once: no such frame went since the
   *  last animation frame that announced nothing. */
  private open = true;
  /** The frame last emitted (the one on screen). */
  private last: { frame: PresentedFrameLike; quality: number } | null = null;
  emitted = 0;
  /** Frames passed over for a newer one due at the same animation frame. */
  coalesced = 0;
  /** Redraw requests answered with the frame on screen (`redraw`). */
  redraws = 0;
  /** The revision of the frame on screen, once one was. */
  get shownRevision(): number | null {
    return this.last?.frame.revision ?? null;
  }

  /** Frames reported and not yet emitted. */
  get pending(): number {
    return this.queue.length;
  }

  private announcements = 0;
  /**
   * Whether a frame is being announced now: its listeners (`frame:rendered`,
   * `overlay`) are running. A redraw they request asks for nothing new, as
   * the frame they react to is the one on screen; answering it with that
   * frame again would announce it once per animation frame for as long as
   * a listener invalidates (the frame loop ignores those requests).
   */
  get announcing(): boolean {
    return this.announcements > 0;
  }

  private announce<T>(fn: () => T): T {
    this.announcements++;
    try {
      return fn();
    } finally {
      this.announcements--;
    }
  }

  /** Why the latest redraw request announced nothing (diagnostics). */
  lastRedraw: string | null = null;
  /** Main-thread time the emissions' listeners took (ms), and the slowest. */
  listenersMs = 0;
  slowestMs = 0;

  constructor(
    private readonly emit: (frame: PresentedFrameLike, quality: number) => void,
    private readonly nextFrame: (fn: (t: number) => void) => void
  ) {}

  push(frame: PresentedFrameLike, quality: number): void {
    if (!frame.timed && this.open && !this.queue.length) {
      this.open = false;
      this.schedule();
      this.send({ frame, quality });
      return;
    }
    this.queue.push({ frame, quality });
    this.schedule();
  }

  private schedule(): void {
    if (this.scheduled) return;
    this.scheduled = true;
    this.nextFrame((t) => {
      this.scheduled = false;
      // Due: drawn on arrival, or by an earlier refresh than this one.
      const due = (f: PresentedFrameLike) => !f.timed || f.rafMs === undefined || !(f.rafMs > t - SAME_REFRESH_MS);
      let index = -1;
      for (let i = this.queue.length - 1; i >= 0; i--) if (due(this.queue[i]!.frame)) { index = i; break; }
      if (index < 0) {
        if (this.queue.length) this.schedule();
        else this.open = true;
        return;
      }
      const next = this.queue[index]!;
      this.coalesced += index;
      this.queue.splice(0, index + 1);
      // A frame drawn on arrival announced here keeps the next one for the
      // next animation frame.
      if (this.queue.length || !this.open) this.schedule();
      this.send(next);
    });
  }

  private send(next: { frame: PresentedFrameLike; quality: number }): void {
    this.last = next;
    this.emitted++;
    const started = performance.now();
    this.announce(() => this.emit(next.frame, next.quality));
    const ms = performance.now() - started;
    this.listenersMs += ms;
    this.slowestMs = Math.max(this.slowestMs, ms);
  }

  /**
   * A Play from rest: today's first animation frame draws the frame at the
   * playhead, and extensions hear `frame:rendered` for it before the run
   * moves. The engine does not draw it again when it is the frame on
   * screen (current revision, view and time), so it is announced again at
   * the next animation frame, ahead of the run's frames. Returns whether
   * it will be.
   */
  replay(current: { revision: number | null; viewSeq: number | null; pts: number | null; frameTicks: number }): boolean {
    const last = this.last;
    if (!last || this.queue.length || !this.matches(last.frame, current)) return false;
    this.replays++;
    // At the next animation frame, ahead of the run's frames.
    this.queue.push({ frame: { ...last.frame, timed: false }, quality: last.quality });
    this.schedule();
    return true;
  }

  /** Whether `frame` is what the engine shows for `current` (same revision, view, and frame time). */
  private matches(frame: PresentedFrameLike, current: { revision: number | null; viewSeq: number | null; pts: number | null; frameTicks: number }): boolean {
    return current.revision !== null && current.revision === frame.revision
      && (current.viewSeq === null || current.viewSeq === frame.viewSeq)
      && (current.pts === null || Math.abs(current.pts - frame.pts) < Math.max(1, current.frameTicks) / 2);
  }

  /** Plays from rest whose frame on screen was announced again (`replay`). */
  replays = 0;

  /**
   * A paused redraw request (today's frame loop drawing on `draw`, which
   * renders and announces the frame again even when nothing changed). The
   * engine presents a new frame only for a new revision, view or time; when
   * none is coming (`current` still matches the frame on screen) the frame
   * on screen is announced again, now, as today's redraw announces it.
   * Returns whether it was.
   */
  redraw(current: { revision: number | null; viewSeq: number | null; pts: number | null; frameTicks: number }, announce: (frame: PresentedFrameLike, quality: number) => boolean): boolean {
    const last = this.last;
    const skip = (why: string) => { this.lastRedraw = why; return false; };
    if (!last) return skip('nothing on screen');
    if (this.queue.length) return skip('a frame is on its way to the screen');
    if (current.revision === null || current.revision !== last.frame.revision) return skip(`revision ${current.revision} after ${last.frame.revision}`);
    if (current.viewSeq !== null && current.viewSeq !== last.frame.viewSeq) return skip(`view ${current.viewSeq} after ${last.frame.viewSeq}`);
    if (current.pts !== null && Math.abs(current.pts - last.frame.pts) >= Math.max(1, current.frameTicks) / 2) return skip(`time ${current.pts} after ${last.frame.pts}`);
    // `announce` declines when the redraw changed what the engine draws (it
    // presents the new frame itself).
    if (!this.announce(() => announce(last.frame, last.quality))) return skip('the producers\' output changed');
    this.lastRedraw = 'announced';
    this.redraws++;
    return true;
  }
}

/** Today's `perf` fields (runtime/install-legacy.ts) from engine stats. */
export type PerfFields = { ms: number; fps: number; draws: number; passes: number; progs: number; raster: number };

/** Engine stats for the status bar and the performance panel. */
export class EngineStatsView {
  latest: RenderStats | null = null;
  private window: { at: number; frames: number } | null = null;
  private fps = 0;

  /** A new document or engine launch has no presented stats yet. */
  reset(): void {
    this.latest = null;
    this.window = null;
    this.fps = 0;
  }

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

  /** The latest session's audio health, or null without engine audio. */
  audio(): AudioHealth | null {
    return audioHealth(this.latest?.audio);
  }
}
