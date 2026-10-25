/** Presents native decoded source planes through Chromium's video path. */
import type { SourceVideoFrame } from '../../../shared/engine-protocol';
type GeneratorTrack = MediaStreamTrack & { readonly writable: WritableStream<VideoFrame> };
type GeneratorConstructor = new (options: { kind: 'video' }) => GeneratorTrack;
const STARTUP_REPLAY_MS = 16;
const STARTUP_TIMEOUT_MS = 5_000;
/** An element that should present but reports nothing (not rendered) cannot
 * hold the monitor: the next frame is written after this many ms anyway. */
const PRESENT_BOUND_MS = 250;
/** WebCodecs buffer layouts of the planes the engine sends (`SourceVideoFrame`).
 * Chromium accepts I420P10 although TypeScript's DOM lib omits it. */
const FORMATS: Record<string, { format: VideoPixelFormat; bytes: (w: number, h: number) => number }> = {
  nv12: { format: 'NV12', bytes: (w, h) => w * h + Math.ceil(w / 2) * Math.ceil(h / 2) * 2 },
  i420a: { format: 'I420A', bytes: (w, h) => 2 * w * h + Math.ceil(w / 2) * Math.ceil(h / 2) * 2 },
  i422: { format: 'I422', bytes: (w, h) => w * h + Math.ceil(w / 2) * h * 2 },
  i420p10: { format: 'I420P10' as VideoPixelFormat, bytes: (w, h) => 2 * (w * h + Math.ceil(w / 2) * Math.ceil(h / 2) * 2) },
  rgba: { format: 'RGBA', bytes: (w, h) => 4 * w * h }
};
const releaseWriter = (writer: WritableStreamDefaultWriter<VideoFrame>): void => {
  try { writer.releaseLock(); } catch { /* A pending abort still owns the lock. */ }
};

export type SourceVideoDescriptor = { video: SourceVideoFrame & { timestamp: number }; data: Uint8ClampedArray; width: number; height: number };

/** The engine only sends tags WebCodecs names (pm-protocol validates them). */
function rawColorSpace(video: SourceVideoFrame): VideoColorSpaceInit {
  return { primaries: video.primaries as VideoColorPrimaries,
    transfer: (video.transfer === 'srgb' ? 'iec61966-2-1' : video.transfer) as VideoTransferCharacteristics,
    matrix: video.matrix as VideoMatrixCoefficients, fullRange: video.range === 'full' };
}

export class SourceVideoPresenter {
  readonly surface = document.createElement('video');
  private track: GeneratorTrack | null = null;
  private writer: WritableStreamDefaultWriter<VideoFrame> | null = null;
  private started: Promise<void> | null = null;
  private disposed = false;
  private readonly disposal = new AbortController();
  private presentedTimestamp = -1;
  private readonly presentation = new Set<() => void>();
  /** Presentation of the last written frame while pending; the next write waits for it. */
  private previous: Promise<void> | null = null;

  constructor() {
    this.surface.muted = true;
    this.surface.playsInline = true;
    this.surface.autoplay = true;
    this.surface.dataset.nativeSource = 'true';
    if (typeof this.surface.requestVideoFrameCallback === 'function') {
      const presented: VideoFrameRequestCallback = (_now, metadata) => {
        if (this.disposed) return;
        this.presentedTimestamp = Math.round(metadata.mediaTime * 1e6);
        for (const wake of this.presentation) wake();
        this.surface.requestVideoFrameCallback(presented);
      };
      this.surface.requestVideoFrameCallback(presented);
    }
  }

  /** Resolves once the element presents the frame stamped `timestamp` (µs)
   * exactly. The next paint writes only after this, so two frames
   * never reach the generated track's sink within one vsync, where it can
   * keep the older. A detached or hidden element presents nothing and does
   * not wait (null). */
  private presentedAt(timestamp: number): Promise<void> | null {
    if (typeof this.surface.requestVideoFrameCallback !== 'function') return null;
    const shown = () => this.disposed || this.presentedTimestamp === timestamp || !this.surface.isConnected || document.hidden;
    if (shown()) return null;
    const presented = new Promise<void>(resolve => {
      const done = () => {
        clearTimeout(timer);
        this.presentation.delete(wake);
        document.removeEventListener('visibilitychange', wake);
        resolve();
      };
      const wake = () => { if (shown()) done(); };
      const timer = setTimeout(done, PRESENT_BOUND_MS);
      this.presentation.add(wake);
      document.addEventListener('visibilitychange', wake);
    });
    void presented.then(() => { if (this.previous === presented) this.previous = null; });
    return presented;
  }

  private ensureWriter(): WritableStreamDefaultWriter<VideoFrame> {
    if (this.writer) return this.writer;
    const Generator = (globalThis as typeof globalThis & { MediaStreamTrackGenerator?: GeneratorConstructor }).MediaStreamTrackGenerator;
    if (!Generator) throw new Error('Native source video presentation is unavailable.');
    const track = new Generator({ kind: 'video' });
    let writer: WritableStreamDefaultWriter<VideoFrame> | null = null;
    try {
      writer = track.writable.getWriter();
      this.surface.srcObject = new MediaStream([track]);
      this.track = track;
      this.writer = writer;
      return writer;
    } catch (error) {
      if (writer) { const held = writer; void held.abort(error).catch(() => {}).finally(() => releaseWriter(held)); }
      track.stop();
      throw error;
    }
  }

  /** Resolves once the frame is written, after the element presented the
   * previous one. */
  async paint(descriptor: SourceVideoDescriptor, signal?: AbortSignal): Promise<void> {
    if (this.disposed) throw new DOMException('Source video presenter is disposed.', 'AbortError');
    if (signal?.aborted) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
    if (!Number.isInteger(descriptor.width) || descriptor.width < 1 || descriptor.width > 16384
        || !Number.isInteger(descriptor.height) || descriptor.height < 1 || descriptor.height > 16384)
      throw new Error('Invalid native source video dimensions.');

    let sourceFrame: VideoFrame | null = null;
    let writtenFrame: VideoFrame | null = null;
    let startupTimer: ReturnType<typeof setTimeout> | null = null;
    let cancelResolve!: () => void;
    let cancelReject!: (error: Error) => void;
    const cancelled = new Promise<void>((resolve, reject) => { cancelResolve = resolve; cancelReject = reject; });
    const onAbort = () => cancelReject(new DOMException('Source video presentation was cancelled.', 'AbortError'));
    signal?.addEventListener('abort', onAbort, { once: true });
    this.disposal.signal.addEventListener('abort', onAbort, { once: true });
    try {
      if (signal?.aborted || this.disposed) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
      if (this.previous) await Promise.race([this.previous, cancelled]);
      if (signal?.aborted || this.disposed) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
      if (!Number.isSafeInteger(descriptor.video.timestamp) || descriptor.video.timestamp < 0) throw new Error('Invalid source delivery timestamp');
      const layout = FORMATS[descriptor.video.format];
      if (!layout) throw new Error('Invalid raw source frame format.');
      if (descriptor.video.visibleWidth !== undefined && (!Number.isInteger(descriptor.video.visibleWidth) || descriptor.video.visibleWidth < 1 || descriptor.video.visibleWidth > descriptor.width)) throw new Error('Invalid source visible width.');
      if (descriptor.data.byteLength !== layout.bytes(descriptor.width, descriptor.height)) throw new Error('Invalid source video plane length.');
      const data = descriptor.data.buffer instanceof ArrayBuffer ? descriptor.data as Uint8ClampedArray<ArrayBuffer> : new Uint8ClampedArray(descriptor.data);
      // This request owns its uncached planes; transfer their buffer once.
      const init: VideoFrameBufferInit & { transfer: ArrayBuffer[] } = { format: layout.format, codedWidth: descriptor.width, codedHeight: descriptor.height,
        timestamp: descriptor.video.timestamp, colorSpace: rawColorSpace(descriptor.video), transfer: [data.buffer],
        ...(descriptor.video.visibleWidth !== undefined ? { visibleRect: { x: 0, y: 0, width: descriptor.video.visibleWidth, height: descriptor.height }, displayWidth: descriptor.video.visibleWidth, displayHeight: descriptor.height } : {}) };
      sourceFrame = new VideoFrame(data, init);
      const timestamp = sourceFrame.timestamp;
      if (!Number.isSafeInteger(timestamp) || timestamp < 0) throw new Error('Invalid source delivery timestamp');
      const writer = this.ensureWriter();
      const visibleWidth = descriptor.video.visibleWidth ?? descriptor.width;
      if (this.surface.width !== visibleWidth) this.surface.width = visibleWidth;
      if (this.surface.height !== descriptor.height) this.surface.height = descriptor.height;

      // Chromium does not retain frames written before the video sink has
      // attached to this new MediaStream. Play is pending until that sink
      // receives a frame, so repeat this held frame until startup settles.
      const playing = this.started ?? (this.started = Promise.resolve(this.surface.play()));
      let played = false;
      void playing.then(() => { played = true; }, () => {});
      const timeout = new Promise<never>((_, reject) => {
        startupTimer = setTimeout(() => reject(new Error('Native source video playback did not start.')), STARTUP_TIMEOUT_MS);
      });
      try {
        for (;;) {
          if (played) break;
          if (this.disposed || signal?.aborted) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
          // Keep the source alive: Chromium can close a VideoFrame passed to
          // the writer, and a dropped startup write must be replayable.
          // Replay the same underlying frame and stamp. Chromium consumes
          // its native timestamp, not a clone's JS-only timestamp override.
          writtenFrame = new VideoFrame(sourceFrame);
          try {
            const writing = writer.write(writtenFrame);
            void writing.catch(() => {});
            await Promise.race([writing, playing.then(() => writing), cancelled, timeout]);
          } finally {
            writtenFrame.close();
            writtenFrame = null;
          }
          if (played) break;
          let tickTimer: ReturnType<typeof setTimeout> | null = null;
          const tick = new Promise<void>(resolve => { tickTimer = setTimeout(resolve, STARTUP_REPLAY_MS); });
          try { await Promise.race([playing, tick, cancelled, timeout]); }
          finally { if (tickTimer !== null) clearTimeout(tickTimer); }
        }
        await Promise.race([playing, cancelled, timeout]);
        this.previous = this.presentedAt(timestamp);
      }
      catch (error) {
        // A rejected play or a detached sink can leave a write waiting for
        // a consumer. Stop it and report failure without awaiting that write.
        if (this.writer === writer && !signal?.aborted && !this.disposed) {
          void writer.abort(error).catch(() => {}).finally(() => releaseWriter(writer));
          this.writer = null;
          this.track?.stop(); this.track = null;
          this.surface.srcObject = null;
          this.started = null;
        }
        throw error;
      }
      if (this.disposed || signal?.aborted) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
    } finally {
      if (startupTimer !== null) clearTimeout(startupTimer);
      signal?.removeEventListener('abort', onAbort);
      this.disposal.signal.removeEventListener('abort', onAbort);
      cancelResolve();
      sourceFrame?.close();
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.disposal.abort();
    for (const wake of [...this.presentation]) wake();
    const writer = this.writer;
    this.writer = null;
    if (writer) void writer.abort(new Error('Source video presenter disposed')).catch(() => {}).finally(() => releaseWriter(writer));
    this.track?.stop(); this.track = null;
    this.started = null;
    this.surface.pause();
    this.surface.srcObject = null;
    this.surface.removeAttribute('src');
    this.surface.remove();
  }
}
