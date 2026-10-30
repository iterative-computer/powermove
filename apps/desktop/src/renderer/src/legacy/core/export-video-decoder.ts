import { mp4VideoSamples, type Mp4VideoSamples } from '../../../../shared/mp4-samples';
import { seekVideoFrame } from './video-frame-decode';

/* Export reads every frame of a clip in order. Seeking a <video> element for
   each one makes Chromium decode again from the previous keyframe, so a screen
   recording with keyframes seconds apart costs tens of decodes per exported
   frame. These decoders walk each clip front to back with WebCodecs instead and
   decode a few frames ahead while the compositor and encoder work. Anything
   they cannot reproduce exactly returns null and keeps the seeking path. */

export type DecodedVideoFrame = { frame: VideoFrame; time: number };
type Ready = { rank: number; frame: VideoFrame };

/** Decoded frames held ahead of the one being exported. */
const READY_AHEAD = 3;
/** Compressed chunks queued inside the decoder at once. */
const QUEUED_CHUNKS = 3;
/** One blob read covers this many bytes of upcoming samples. */
const READ_BYTES = 8 * 1024 * 1024;
const FRAME_TIMEOUT_MS = 15_000;
/** Chromium's media clock is in microseconds. */
const EPSILON = 1e-6;
/* Untagged video leaves the YUV matrix to Chromium, and <video> and WebCodecs
   do not always choose alike (small software-decoded clips differ). Each clip
   is checked against its element once and may switch to the other standard. */
const COLOR_CANDIDATES: (VideoColorSpaceInit | undefined)[] = [undefined,
  { primaries: 'smpte170m', transfer: 'smpte170m', matrix: 'smpte170m', fullRange: false },
  { primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', fullRange: false }];

class SequentialClip {
  private readonly decoder: VideoDecoder;
  /** Presentation rank → decode index, for frames on the visible timeline. */
  private readonly order: Uint32Array;
  private readonly rankByTimestamp = new Map<number, number>();
  private readonly timestamps: Float64Array;
  private next = 0;
  private flushing = false;
  private drained = false;
  private generation = 0;
  private target = -1;
  private ready: Ready[] = [];
  private current: Ready | null = null;
  private failure: unknown = null;
  private pumping: Promise<void> | null = null;
  private waiters = new Set<() => void>();
  private cache: { start: number; bytes: Uint8Array } | null = null;
  private closed = false;
  private color = 0;

  constructor(private readonly samples: Mp4VideoSamples, private config: VideoDecoderConfig,
    private readonly blob: Blob, private readonly width: number, private readonly height: number) {
    const count = samples.pts.length;
    this.timestamps = new Float64Array(count);
    const shown: number[] = [];
    for (let i = 0; i < count; i++) {
      this.timestamps[i] = Math.round(samples.pts[i]! * 1e6);
      // Frames before an edit's start only feed the decoder.
      if (samples.pts[i]! >= -EPSILON) shown.push(i);
    }
    shown.sort((a, b) => samples.pts[a]! - samples.pts[b]!);
    this.order = Uint32Array.from(shown);
    shown.forEach((index, rank) => this.rankByTimestamp.set(this.timestamps[index]!, rank));
    this.decoder = new VideoDecoder({
      output: frame => this.output(frame),
      error: error => this.fail(error),
    });
    this.decoder.addEventListener?.('dequeue', () => { this.pump(); this.notify(); });
    this.decoder.configure(config);
  }

  static async open(blob: Blob, width: number, height: number): Promise<SequentialClip | null> {
    if (typeof VideoDecoder === 'undefined' || typeof EncodedVideoChunk === 'undefined') return null;
    const samples = await mp4VideoSamples(blob);
    if (!samples || !samples.pts.length) return null;
    const config: VideoDecoderConfig = {
      codec: samples.codec, codedWidth: samples.width, codedHeight: samples.height,
      ...(samples.description ? { description: samples.description } : {}),
      ...(samples.colorSpace ? { colorSpace: samples.colorSpace } : {}),
      optimizeForLatency: true,
    };
    const support = await VideoDecoder.isConfigSupported(config).catch(() => null);
    if (!support?.supported) return null;
    return new SequentialClip(samples, config, blob, width, height);
  }

  private pts(rank: number) { return this.samples.pts[this.order[rank]!]!; }

  /** The frame <video> shows at `time`: the last one starting at or before it. */
  private select(time: number): number {
    let low = 0, high = this.order.length - 1, found = 0;
    while (low <= high) {
      const middle = (low + high) >> 1;
      if (this.pts(middle) <= time + EPSILON) { found = middle; low = middle + 1; } else high = middle - 1;
    }
    return found;
  }

  /** The sync sample decoding must start from to reach `index`. */
  private keyFor(index: number): number {
    const { keys, pts } = this.samples;
    for (let i = index; i > 0; i--) if (keys[i] && pts[i]! <= pts[index]! + EPSILON) return i;
    return 0;
  }

  private output(frame: VideoFrame) {
    const rank = this.rankByTimestamp.get(frame.timestamp);
    if (this.closed || rank === undefined || rank < this.target) frame.close();
    else {
      let at = this.ready.length;
      while (at > 0 && this.ready[at - 1]!.rank > rank) at--;
      this.ready.splice(at, 0, { rank, frame });
    }
    this.pump();
    this.notify();
  }

  private fail(error: unknown) {
    if (!this.failure) this.failure = error instanceof Error ? error : new Error(String(error));
    this.notify();
  }

  private notify() {
    const waiters = [...this.waiters];
    this.waiters.clear();
    for (const wake of waiters) wake();
  }

  private restart(from: number) {
    this.generation++;
    for (const item of this.ready) item.frame.close();
    this.ready = [];
    this.decoder.reset();
    this.decoder.configure(this.config);
    this.next = from;
    this.flushing = false; this.drained = false;
  }

  private async bytes(index: number): Promise<Uint8Array> {
    const { offsets, sizes } = this.samples;
    const start = offsets[index]!, end = start + sizes[index]!;
    const cache = this.cache;
    if (!cache || start < cache.start || end > cache.start + cache.bytes.length) {
      // Samples are mostly contiguous (interleaved with audio): read ahead in one slice.
      let stop = end;
      for (let i = index + 1; i < offsets.length && i < index + 64; i++) {
        const sampleEnd = offsets[i]! + sizes[i]!;
        if (offsets[i]! < start || sampleEnd - start > READ_BYTES) break;
        stop = Math.max(stop, sampleEnd);
      }
      this.cache = { start, bytes: new Uint8Array(await this.blob.slice(start, stop).arrayBuffer()) };
      return this.cache.bytes.subarray(0, end - start);
    }
    return cache.bytes.subarray(start - cache.start, end - cache.start);
  }

  /** Keep a few chunks queued and a few frames decoded ahead of the target. */
  private pump() {
    if (this.pumping || this.closed || this.failure) return;
    const generation = this.generation;
    // Wake waiters only on progress. Outputs and dequeues re-enter this pump,
    // and a pump that merely found the queue full must not spin the waiters.
    let progressed = false;
    this.pumping = (async () => {
      while (generation === this.generation && !this.failure && this.decoder.state === 'configured') {
        const count = this.samples.pts.length;
        if (this.next < count) {
          if (this.ready.length >= READY_AHEAD || this.decoder.decodeQueueSize >= QUEUED_CHUNKS) return;
          const index = this.next;
          const data = await this.bytes(index);
          if (generation !== this.generation || this.decoder.state !== 'configured') return;
          this.decoder.decode(new EncodedVideoChunk({ type: this.samples.keys[index] ? 'key' : 'delta', timestamp: this.timestamps[index]!, data }));
          this.next = index + 1;
          progressed = true;
          continue;
        }
        if (this.flushing) return;
        // The decoder holds its last reordered frames until told the stream ended.
        this.flushing = true;
        await this.decoder.flush().then(() => { if (generation === this.generation) this.drained = true; }, () => undefined);
        progressed = true;
        return;
      }
    })().catch(error => this.fail(error)).finally(() => { this.pumping = null; if (progressed) this.notify(); });
  }

  private wait(signal: AbortSignal | undefined, deadline: number): Promise<void> {
    return new Promise((resolve, reject) => {
      const done = () => { clearTimeout(timer); signal?.removeEventListener('abort', aborted); this.waiters.delete(done); resolve(); };
      const aborted = () => { clearTimeout(timer); this.waiters.delete(done); reject(signal!.reason ?? new DOMException('Frame capture cancelled', 'AbortError')); };
      const timer = setTimeout(() => { this.waiters.delete(done); signal?.removeEventListener('abort', aborted); reject(new Error('Video decoder timed out')); }, Math.max(0, deadline - performance.now()));
      if (signal?.aborted) { aborted(); return; }
      signal?.addEventListener('abort', aborted, { once: true });
      this.waiters.add(done);
    });
  }

  async frameAt(time: number, signal?: AbortSignal): Promise<DecodedVideoFrame> {
    if (this.failure) throw this.failure;
    const rank = this.select(time);
    if (this.current?.rank === rank) return { frame: this.current.frame, time: this.pts(rank) };
    const index = this.order[rank]!, key = this.keyFor(index);
    // Going back, or a keyframe ahead of what was queued: start again from it.
    if (this.current && this.current.rank > rank || this.ready[0] && this.ready[0].rank > rank || key > this.next
        || this.drained && !this.ready.some(item => item.rank === rank)) this.restart(key);
    this.target = rank;
    const deadline = performance.now() + FRAME_TIMEOUT_MS;
    for (;;) {
      if (this.failure) throw this.failure;
      while (this.ready[0] && this.ready[0].rank < rank) this.ready.shift()!.frame.close();
      if (this.ready[0]?.rank === rank) break;
      if (this.ready[0] || this.drained && !this.pumping) {
        this.fail(new Error(`Video decoder skipped the frame at ${this.pts(rank).toFixed(6)}s`));
        continue;
      }
      this.pump();
      await this.wait(signal, deadline);
    }
    const taken = this.ready.shift()!;
    if (taken.frame.displayWidth !== this.width || taken.frame.displayHeight !== this.height) {
      taken.frame.close();
      this.fail(new Error(`Decoded ${taken.frame.displayWidth}×${taken.frame.displayHeight}, expected ${this.width}×${this.height}`));
      throw this.failure;
    }
    this.current?.frame.close();
    this.current = taken;
    this.pump();
    return { frame: taken.frame, time: this.pts(rank) };
  }

  /** Decode with the next candidate color description; false when none remain.
   * A container tag is authoritative, so tagged clips have no alternative. */
  nextColor(): boolean {
    if (this.samples.colorSpace || this.color + 1 >= COLOR_CANDIDATES.length) return false;
    const colorSpace = COLOR_CANDIDATES[++this.color];
    const { colorSpace: _, ...config } = this.config;
    this.config = { ...config, ...(colorSpace ? { colorSpace } : {}) };
    this.current?.frame.close();
    this.current = null;
    this.restart(0);
    return true;
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    this.generation++;
    for (const item of this.ready) item.frame.close();
    this.ready = [];
    this.current?.frame.close();
    this.current = null;
    if (this.decoder.state !== 'closed') this.decoder.close();
    this.notify();
  }
}

type Clip = { url: string; clip: Promise<SequentialClip | null>; failed?: boolean; verified?: boolean; used: number };
/** Requests after which an unused decoder (a clip that has ended) is closed.
 * Hardware decoder sessions are limited; one reopens if the clip returns. */
const IDLE_REQUESTS = 240;

function pixels(source: CanvasImageSource, sx: number, sy: number, width: number, height: number, scale = 1) {
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(width * scale)); canvas.height = Math.max(1, Math.round(height * scale));
  const context = canvas.getContext('2d', { willReadFrequently: true })!;
  context.drawImage(source, sx, sy, width, height, 0, 0, canvas.width, canvas.height);
  return context.getImageData(0, 0, canvas.width, canvas.height).data;
}
/** Neutral (gray) pixels convert identically under every YUV matrix. */
function colorful(frame: VideoFrame): boolean {
  const data = pixels(frame, 0, 0, frame.displayWidth, frame.displayHeight, 48 / Math.max(frame.displayWidth, frame.displayHeight));
  for (let i = 0; i < data.length; i += 4) {
    const r = data[i]!, g = data[i + 1]!, b = data[i + 2]!;
    if (Math.max(r, g, b) - Math.min(r, g, b) > 24) return true;
  }
  return false;
}
/** The decoded frame matches what the element shows, at native scale. */
function sameAsElement(frame: VideoFrame, element: HTMLVideoElement): boolean {
  const width = Math.min(frame.displayWidth, 512), height = Math.min(frame.displayHeight, 512);
  const x = (frame.displayWidth - width) >> 1, y = (frame.displayHeight - height) >> 1;
  const decoded = pixels(frame, x, y, width, height), shown = pixels(element, x, y, width, height);
  for (let i = 0; i < decoded.length; i++) if (Math.abs(decoded[i]! - shown[i]!) > 2) return false;
  return true;
}

/** The bytes the element plays. Stored assets release their Blob after
 * saving; a blob: URL still resolves to the same data without copying it. */
async function sourceBlob(asset: any): Promise<Blob | null> {
  if (asset.persistBlob instanceof Blob) return asset.persistBlob;
  if (!/^blob:/.test(asset.url)) return null;
  const response = await fetch(asset.url);
  return response.ok ? await response.blob() : null;
}

/** Per-export decoders, one per layer instance of a clip so clips that share
 * media at different times do not restart each other. */
export class ExportVideoDecoders {
  private clips = new Map<string, Clip>();
  private disposed = false;
  /** How each layer's latest frame was produced. */
  private routes = new Map<string, 'decoder' | 'seek'>();
  private requests = 0;
  readonly stats = { decoded: 0, seeked: 0, fallbacks: [] as string[] };

  /** The decoded frame for a layer, or null to seek the asset's element instead. */
  async frame(asset: any, layerKey: string, time: number, signal?: AbortSignal): Promise<DecodedVideoFrame | null> {
    const route = layerKey;
    const element = asset?.el;
    if (this.disposed || asset?.imageSequence || typeof asset?.url !== 'string' || typeof HTMLVideoElement === 'undefined'
        || !(element instanceof HTMLVideoElement) || !asset.url || (element.currentSrc || element.src) !== asset.url
        || !(element.videoWidth > 0 && element.videoHeight > 0)) {
      this.routes.set(route, 'seek'); this.stats.seeked++;
      return null;
    }
    const key = `${asset.id}\u0000${layerKey}`, request = ++this.requests;
    for (const [other, idle] of this.clips) {
      if (other !== key && request - idle.used > IDLE_REQUESTS) { this.clips.delete(other); void idle.clip.then(clip => clip?.close()); }
    }
    let entry = this.clips.get(key);
    // Relinked or converted media gets a new URL and a fresh decoder.
    if (entry && entry.url !== asset.url) { void entry.clip.then(clip => clip?.close()); entry = undefined; }
    if (!entry) {
      const width = element.videoWidth, height = element.videoHeight;
      entry = { url: asset.url, clip: sourceBlob(asset).then(blob => blob && SequentialClip.open(blob, width, height)).catch(() => null), used: request };
      this.clips.set(key, entry);
    }
    entry.used = request;
    const clip = entry.failed ? null : await entry.clip;
    if (this.disposed) { clip?.close(); throw new DOMException('Frame capture cancelled', 'AbortError'); }
    if (clip) {
      try {
        let frame = await clip.frameAt(time, signal);
        // Verify once against the element, on the first frame whose color
        // conversion can differ; this also proves the timeline mapping.
        if (!entry.verified && colorful(frame.frame)) {
          await seekVideoFrame(element, time, { signal, tolerance: .0005, timeoutMs: 15_000 });
          while (!sameAsElement(frame.frame, element)) {
            if (!clip.nextColor()) throw new Error(`Decoded frame at ${frame.time.toFixed(6)}s differs from the video element`);
            frame = await clip.frameAt(time, signal);
          }
          entry.verified = true;
        }
        this.routes.set(route, 'decoder'); this.stats.decoded++;
        return frame;
      } catch (error) {
        if (signal?.aborted) throw error;
        // One failure retires this clip's decoder; seeking stays correct.
        entry.failed = true; clip.close();
        this.stats.fallbacks.push(`${asset.name || asset.id}: ${error instanceof Error ? error.message : String(error)}`);
        console.warn('[export] Sequential video decoding stopped; seeking instead.', error);
      }
    }
    this.routes.set(route, 'seek'); this.stats.seeked++;
    return null;
  }

  /** True when every listed layer's latest frame came from a sequential decoder. */
  decoding(layerKeys: Iterable<string>): boolean {
    for (const key of layerKeys) if (this.routes.get(key) !== 'decoder') return false;
    return true;
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    for (const entry of this.clips.values()) void entry.clip.then(clip => clip?.close());
    this.clips.clear();
  }
}
