/** One-shot decoded source video planes presented through Electron's video path. */
import { randomUUID } from 'node:crypto';
import type { SourceVideoFrame } from '../../shared/engine-protocol';
import type { SourceSurfaceArgs } from '../../shared/frame-bridge';

export const SOURCE_SURFACE_BYTES_LIMIT = 512 * 1024 * 1024;
export const SOURCE_SURFACES_PER_WINDOW = 8;
export const SOURCE_SURFACE_RECLAIM_MS = 2_000;

export type NativeSourceSurface = { readonly ioSurface: Buffer; readonly allocBytes: number; release(): void };
export type SourceVideoSurfaceConstructor = new (
  width: number, height: number, format: SourceVideoFrame['format'], bytes: Uint8Array, fullRange: boolean
) => NativeSourceSurface;
export type ImportedSourceTexture = { release(): void };
export type SourceFrameTarget = { readonly detached?: boolean; isDestroyed?(): boolean };
export type SourceSharedTexture = {
  importSharedTexture(options: {
    textureInfo: {
      pixelFormat: SourceVideoFrame['format'];
      codedSize: { width: number; height: number };
      visibleRect: { x: number; y: number; width: number; height: number };
      colorSpace: { primaries: string; transfer: string; matrix: string; range: string };
      handle: { ioSurface: Buffer };
    };
    allReferencesReleased(): void;
  }): ImportedSourceTexture;
  sendSharedTexture(options: { frame: never; importedSharedTexture: ImportedSourceTexture }, args: SourceSurfaceArgs): Promise<void>;
};
export type SourceSurfaceTimers = { set(callback: () => void, delay: number): unknown; clear(handle: unknown): void };
const defaultTimers: SourceSurfaceTimers = {
  set: (callback, delay) => {
    const timer = setTimeout(callback, delay);
    timer.unref?.();
    return timer;
  },
  clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>)
};

export type SourceSurfaceOptions = {
  addon: { SourceVideoSurface: SourceVideoSurfaceConstructor };
  sharedTexture: SourceSharedTexture;
  /** windowId is webContents.id; resolve to that window's mainFrame. */
  resolveFrame(windowId: number): SourceFrameTarget | null;
  timers?: SourceSurfaceTimers;
};

type Entry = {
  token: string;
  windowId: number;
  bytes: number;
  native: NativeSourceSurface;
  imported: ImportedSourceTexture | null;
  mainReleased: boolean;
  finished: boolean;
  gone: boolean;
  reclaim: unknown | null;
};

const PRIMARIES = new Set(['unspecified', 'bt709', 'bt470m', 'bt470bg', 'smpte170m', 'smpte240m', 'film', 'bt2020', 'smptest428-1', 'smptest431-2', 'p3']);
const TRANSFER = new Set(['unspecified', 'bt709', 'gamma18', 'gamma22', 'gamma24', 'gamma28', 'smpte170m', 'smpte240m', 'linear', 'srgb', 'pq', 'hlg']);
const MATRIX = new Set(['unspecified', 'bt709', 'fcc', 'bt470bg', 'smpte170m', 'smpte240m', 'ycocg', 'bt2020-ncl']);

/** Match pm-protocol's tightly packed Y + interleaved UV length, including odd sizes. */
export function sourceVideoByteLength(width: number, height: number, video: SourceVideoFrame): number {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1 || width > 16384 || height > 16384)
    throw new Error('Invalid source surface dimensions');
  if (!video || typeof video !== 'object' || !['nv12', 'nv16', 'p010le', 'i420a'].includes(video.format)
      || !PRIMARIES.has(video.primaries) || !TRANSFER.has(video.transfer)
      || !MATRIX.has(video.matrix) || !['limited', 'full'].includes(video.range))
    throw new Error('Invalid source video format or colour tags');
  if (video.visibleWidth !== undefined && (!Number.isSafeInteger(video.visibleWidth) || video.visibleWidth < 1 || video.visibleWidth > width))
    throw new Error('Invalid source visible width');
  const chromaHeight = video.format === 'nv16' ? height : Math.ceil(height / 2);
  const bytesPerSample = video.format === 'p010le' ? 2 : 1;
  return (width * height + Math.ceil(width / 2) * 2 * chromaHeight) * bytesPerSample + (video.format === 'i420a' ? width * height : 0);
}

export class SourceSurfaceService {
  private readonly entries = new Map<string, Entry>();
  private readonly timers: SourceSurfaceTimers;
  private reservedBytes = 0;
  private disposed = false;

  constructor(private readonly options: SourceSurfaceOptions) {
    this.timers = options.timers ?? defaultTimers;
  }

  get held(): number { return this.entries.size; }
  get bytes(): number { return this.reservedBytes; }

  /** Import one native YUV IOSurface and send it to the target preload once. */
  async present(windowId: number, width: number, height: number, video: SourceVideoFrame, data: Uint8Array): Promise<SourceSurfaceArgs> {
    if (this.disposed) throw new Error('Source surfaces are disposed');
    if (video.format === 'i420a') throw new Error('Alpha source planes require their I420A video frame.');
    if (!Number.isSafeInteger(windowId) || windowId < 1) throw new Error('Invalid source window');
    const byteLength = sourceVideoByteLength(width, height, video);
    if (!(data instanceof Uint8Array) || data.byteLength !== byteLength) throw new Error('Incorrect source video plane length');
    if (byteLength > SOURCE_SURFACE_BYTES_LIMIT || this.reservedBytes + byteLength > SOURCE_SURFACE_BYTES_LIMIT)
      throw new Error('Source surface memory budget exceeded');
    if ([...this.entries.values()].filter(entry => entry.windowId === windowId).length >= SOURCE_SURFACES_PER_WINDOW)
      throw new Error('Source surface window budget exceeded');
    const frame = this.options.resolveFrame(windowId);
    if (!frame || frame.detached || frame.isDestroyed?.()) throw new Error('Source window is gone');

    const native = new this.options.addon.SourceVideoSurface(width, height, video.format, data, video.range === 'full');
    if (!Buffer.isBuffer(native.ioSurface) || native.ioSurface.length === 0) {
      native.release();
      throw new Error('Source surface did not expose an IOSurface handle');
    }
    // IOSurface planes can have padded row strides. Account for the retained
    // allocation reported by the native object, not the packed YUV payload.
    const allocBytes = native.allocBytes;
    if (!Number.isSafeInteger(allocBytes) || allocBytes < byteLength) {
      native.release();
      throw new Error('Source surface reported an invalid allocation size');
    }
    if (this.reservedBytes + allocBytes > SOURCE_SURFACE_BYTES_LIMIT) {
      native.release();
      throw new Error('Source surface memory budget exceeded');
    }
    const token = randomUUID();
    const args: SourceSurfaceArgs = { kind: 'powermove-source-surface', token, width: video.visibleWidth ?? width, height };
    const entry: Entry = { token, windowId, bytes: allocBytes, native, imported: null, mainReleased: false,
      finished: false, gone: false, reclaim: null };
    this.entries.set(token, entry);
    this.reservedBytes += allocBytes;
    try {
      entry.imported = this.options.sharedTexture.importSharedTexture({
        textureInfo: { pixelFormat: video.format, codedSize: { width, height },
          visibleRect: { x: 0, y: 0, width: video.visibleWidth ?? width, height },
          colorSpace: { primaries: video.primaries, transfer: video.transfer, matrix: video.matrix, range: video.range },
          handle: { ioSurface: native.ioSurface } },
        allReferencesReleased: () => this.finish(entry)
      });
      await this.options.sharedTexture.sendSharedTexture({ frame: frame as never, importedSharedTexture: entry.imported }, args);
      if (entry.gone || this.disposed) throw new Error('Source window went away during transfer');
      this.releaseMain(entry);
      return args;
    } catch (error) {
      this.releaseMain(entry);
      // A send timeout can still have delivered the renderer reference. Keep
      // the native IOSurface until Electron reports all references released.
      if (!entry.imported) this.finish(entry);
      throw error;
    }
  }

  /** Navigation/crash/close: renderer notices may never arrive. */
  windowGone(windowId: number): void {
    for (const entry of this.entries.values()) {
      if (entry.windowId !== windowId || entry.finished || entry.gone) continue;
      entry.gone = true;
      this.releaseMain(entry);
      if (entry.reclaim === null) entry.reclaim = this.timers.set(() => this.finish(entry), SOURCE_SURFACE_RECLAIM_MS);
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const entry of [...this.entries.values()]) {
      this.releaseMain(entry);
      if (!entry.imported) this.finish(entry);
      else if (entry.reclaim === null) entry.reclaim = this.timers.set(() => this.finish(entry), SOURCE_SURFACE_RECLAIM_MS);
    }
  }

  private releaseMain(entry: Entry): void {
    if (entry.mainReleased || !entry.imported) return;
    entry.mainReleased = true;
    try { entry.imported.release(); }
    catch {
      // The renderer may already have received the import. Hold the native
      // IOSurface for the same grace used when its page disappears.
      if (entry.reclaim === null) entry.reclaim = this.timers.set(() => this.finish(entry), SOURCE_SURFACE_RECLAIM_MS);
    }
  }

  private finish(entry: Entry): void {
    if (entry.finished) return;
    entry.finished = true;
    if (entry.reclaim !== null) this.timers.clear(entry.reclaim);
    this.entries.delete(entry.token);
    this.reservedBytes -= entry.bytes;
    entry.native.release();
  }
}
