/** Presents native decoded source planes through Chromium's video path. */
import type { SourceVideoFrame } from '../../../shared/engine-protocol';

type GeneratorTrack = MediaStreamTrack & { readonly writable: WritableStream<VideoFrame> };
type GeneratorConstructor = new (options: { kind: 'video' }) => GeneratorTrack;
const STARTUP_REPLAY_MS = 16;
const STARTUP_TIMEOUT_MS = 5_000;
/** WebCodecs buffer formats of pm-protocol's SourceVideoFrame layouts.
 * Chromium accepts I420P10 although TypeScript's DOM lib omits it. */
const FORMATS: Record<string, { format: VideoPixelFormat | 'I420P10'; chroma: 1 | 2; bytes: 1 | 2; alpha: boolean }> = {
  nv12: { format: 'NV12', chroma: 2, bytes: 1, alpha: false },
  i420a: { format: 'I420A', chroma: 2, bytes: 1, alpha: true },
  i422: { format: 'I422', chroma: 1, bytes: 1, alpha: false },
  i420p10: { format: 'I420P10', chroma: 2, bytes: 2, alpha: false }
};
const releaseWriter = (writer: WritableStreamDefaultWriter<VideoFrame>): void => {
  try { writer.releaseLock(); } catch { /* A pending abort still owns the lock. */ }
};

export type SourceVideoDescriptor = { video: SourceVideoFrame; data: Uint8ClampedArray; width: number; height: number };

function rawColorSpace(video: SourceVideoFrame): VideoColorSpaceInit {
  const primaries = ({ p3: 'smpte432', 'smptest431-2': 'smpte431' } as Record<string, string>)[video.primaries] ?? video.primaries;
  const transfer = ({ srgb: 'iec61966-2-1', pq: 'smpte2084', hlg: 'arib-std-b67' } as Record<string, string>)[video.transfer] ?? video.transfer;
  return { primaries: primaries === 'unspecified' ? null : primaries as VideoColorPrimaries, transfer: transfer === 'unspecified' ? null : transfer as VideoTransferCharacteristics,
    matrix: video.matrix === 'unspecified' ? null : video.matrix as VideoMatrixCoefficients, fullRange: video.range === 'full' };
}

export class SourceVideoPresenter {
  readonly surface = document.createElement('video');
  private track: GeneratorTrack | null = null;
  private writer: WritableStreamDefaultWriter<VideoFrame> | null = null;
  private started: Promise<void> | null = null;
  private lastTimestamp = -1;
  private disposed = false;
  private readonly disposal = new AbortController();

  constructor() {
    this.surface.muted = true;
    this.surface.playsInline = true;
    this.surface.autoplay = true;
    this.surface.dataset.nativeSource = 'true';
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

  /** Resolves after the frame has been accepted by the video track. */
  async paint(descriptor: SourceVideoDescriptor, signal?: AbortSignal): Promise<void> {
    if (this.disposed) throw new DOMException('Source video presenter is disposed.', 'AbortError');
    if (signal?.aborted) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
    if (!Number.isInteger(descriptor.width) || descriptor.width < 1 || descriptor.width > 16384
        || !Number.isInteger(descriptor.height) || descriptor.height < 1 || descriptor.height > 16384)
      throw new Error('Invalid native source video dimensions.');

    let sourceFrame: VideoFrame | null = null;
    let stampedFrame: VideoFrame | null = null;
    let startupTimer: ReturnType<typeof setTimeout> | null = null;
    let cancelResolve!: () => void;
    let cancelReject!: (error: Error) => void;
    const cancelled = new Promise<void>((resolve, reject) => { cancelResolve = resolve; cancelReject = reject; });
    const onAbort = () => cancelReject(new DOMException('Source video presentation was cancelled.', 'AbortError'));
    signal?.addEventListener('abort', onAbort, { once: true });
    this.disposal.signal.addEventListener('abort', onAbort, { once: true });
    try {
      if (signal?.aborted || this.disposed) throw new DOMException('Source video presentation was cancelled.', 'AbortError');
      const layout = FORMATS[descriptor.video.format];
      if (!layout) throw new Error('Invalid raw source frame format.');
      if (descriptor.video.visibleWidth !== undefined && (!Number.isInteger(descriptor.video.visibleWidth) || descriptor.video.visibleWidth < 1 || descriptor.video.visibleWidth > descriptor.width)) throw new Error('Invalid source visible width.');
      const luma = descriptor.width * descriptor.height;
      const expected = (luma + 2 * Math.ceil(descriptor.width / 2) * Math.ceil(descriptor.height / layout.chroma)) * layout.bytes + (layout.alpha ? luma : 0);
      if (descriptor.data.byteLength !== expected) throw new Error('Invalid source video plane length.');
      const data = descriptor.data.buffer instanceof ArrayBuffer ? descriptor.data as Uint8ClampedArray<ArrayBuffer> : new Uint8ClampedArray(descriptor.data);
      // This request owns its uncached planes; transfer their buffer once.
      const init: VideoFrameBufferInit & { transfer: ArrayBuffer[] } = { format: layout.format as VideoPixelFormat, codedWidth: descriptor.width, codedHeight: descriptor.height,
        timestamp: Math.round(performance.now() * 1000), colorSpace: rawColorSpace(descriptor.video), transfer: [data.buffer],
        ...(descriptor.video.visibleWidth !== undefined ? { visibleRect: { x: 0, y: 0, width: descriptor.video.visibleWidth, height: descriptor.height }, displayWidth: descriptor.video.visibleWidth, displayHeight: descriptor.height } : {}) };
      sourceFrame = new VideoFrame(data, init);
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
          const timestamp = this.lastTimestamp = Math.max(this.lastTimestamp + 1, Math.round(performance.now() * 1000));
          stampedFrame = new VideoFrame(sourceFrame, { timestamp });
          try {
            const writing = writer.write(stampedFrame);
            void writing.catch(() => {});
            await Promise.race([writing, playing.then(() => writing), cancelled, timeout]);
          } finally {
            stampedFrame.close();
            stampedFrame = null;
          }
          if (played) break;
          let tickTimer: ReturnType<typeof setTimeout> | null = null;
          const tick = new Promise<void>(resolve => { tickTimer = setTimeout(resolve, STARTUP_REPLAY_MS); });
          try { await Promise.race([playing, tick, cancelled, timeout]); }
          finally { if (tickTimer !== null) clearTimeout(tickTimer); }
        }
        await Promise.race([playing, cancelled, timeout]);
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
