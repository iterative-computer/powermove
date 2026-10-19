/*
 * Level 1 SVG rasters drawn by Chromium for the native engine (decision S1;
 * docs/engine/vector.md, "Level 1 SVG images"; RFC 16, "SVG rasters").
 *
 * Today a `compat` level 1 SVG image is `PM.rasterSvgAsset`: the asset's
 * decoded `<img>` drawn with `drawImage(img, 0, 0, w, h)` into a cleared
 * canvas at the bucket `svgRasterDimensions` picks, then uploaded by the
 * compositor's `texFor` with `UNPACK_PREMULTIPLY_ALPHA_WEBGL`. The engine
 * composites that texture natively but does not emulate Chromium's SVG
 * rasterization: this module runs the same code and hands the engine the
 * bytes the texture holds, read back from a WebGL2 texture uploaded with
 * the same flags (`SvgRasterCanvas`). The 2D canvas is never read with
 * `getImageData` (that unpremultiplies, and can move the canvas off the
 * GPU).
 *
 * `SvgRasters` answers the engine for the window:
 *
 * - **Viewer frames** (`svgRequests` on the session's port): each request
 *   names an asset, the SHA-256 of its SVG bytes, and a bucket. The asset's
 *   decoded `<img>` is drawn only when the bytes it decoded hash the same
 *   (otherwise `stale`, and the engine asks again later), and the pixels go
 *   back in row bands (`svgPixels`), once per (content, bucket): the engine
 *   keeps them and never asks again for that key while it runs.
 * - **Snapshots** (`needsRenderer.svg`): drawn on the calling thread
 *   (`drawSync`, with the content hashes this module computed ahead,
 *   `prime`, or, for an asset imported or replaced in the same task, read
 *   and hashed there), posted, and named in the next ask's `uploaded`.
 * - **Job engine frames** (`compatFrame` with source `svgImage`, asked by
 *   main): `produce`, asynchronously.
 *
 * Nothing here needs Electron: a plain Chromium page (the hidden host's
 * on-demand Chromium, the capture tools) rasterizes SVG bytes the same way
 * (`decodeSvg`, `SvgRasterCanvas`).
 */
import type { CompatFrameRequest, SvgRasterKey, SvgRasterRequest } from '../../../shared/engine-protocol';

/** Largest band of one message (pm_protocol MAX_SVG_BAND_BYTES). */
export const MAX_SVG_BAND_BYTES = 8 * 1024 * 1024;
/** Largest bucket edge (`svgRasterDimensions`' limit, MAX_SVG_EDGE). */
export const MAX_SVG_EDGE = 8192;
/** Rasters drawn at once for the viewer's asks (each holds a canvas). */
export const MAX_CONCURRENT = 4;
/** Viewer asks `SvgRasters.asks` keeps. */
export const MAX_ASK_LOG = 256;

export type SvgFailureReason = 'missing' | 'stale' | 'decode' | 'size' | 'unavailable';

export class SvgRasterError extends Error {
  constructor(readonly reason: SvgFailureReason, message: string) {
    super(message);
  }
}

/** `getCanvas` (legacy/gl/raster.ts). */
function todaysCanvas(w: number, h: number): HTMLCanvasElement {
  const cv = window.document.createElement('canvas');
  cv.width = Math.max(1, Math.ceil(w));
  cv.height = Math.max(1, Math.ceil(h));
  return cv;
}

export interface SvgRasterCanvasOptions {
  /** The bucket canvas (default: today's `getCanvas`). */
  createCanvas?: (width: number, height: number) => HTMLCanvasElement;
  /** The WebGL2 context the canvas is uploaded through (default: one of
   *  this module's own, with the compositor's attributes). */
  gl?: () => WebGL2RenderingContext | null;
}

/**
 * Today's `rasterSvgAsset` followed by the compositor's `texFor` upload,
 * read back: premultiplied RGBA8, top-down, `width × height × 4` bytes.
 */
export class SvgRasterCanvas {
  private context: WebGL2RenderingContext | null = null;

  constructor(private readonly options: SvgRasterCanvasOptions = {}) {}

  private gl(): WebGL2RenderingContext {
    if (this.options.gl) {
      const gl = this.options.gl();
      if (!gl) throw new SvgRasterError('unavailable', 'No WebGL2 context to read SVG rasters back with.');
      return gl;
    }
    if (!this.context || this.context.isContextLost()) {
      // The compositor's attributes (GL.init); the drawing buffer is never used.
      this.context = window.document.createElement('canvas').getContext('webgl2', {
        alpha: false, antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: false, powerPreference: 'high-performance'
      });
    }
    if (!this.context) throw new SvgRasterError('unavailable', 'This window has no WebGL2 to read SVG rasters back with.');
    return this.context;
  }

  draw(image: CanvasImageSource, width: number, height: number): Uint8Array {
    const w = Math.max(1, Math.round(width)), h = Math.max(1, Math.round(height));
    if (w > MAX_SVG_EDGE || h > MAX_SVG_EDGE) throw new SvgRasterError('size', `A ${w}×${h} SVG raster is over ${MAX_SVG_EDGE} px.`);
    const cv = (this.options.createCanvas ?? todaysCanvas)(w, h);
    try {
      // PM.rasterSvgAsset, statement for statement.
      const context = cv.getContext('2d');
      if (!context) throw new SvgRasterError('unavailable', 'Could not rasterize this SVG file');
      context.clearRect(0, 0, w, h);
      context.drawImage(image, 0, 0, w, h);
      // texFor: the same pixel store flags, then the texture read back.
      const gl = this.gl();
      const texture = gl.createTexture();
      const framebuffer = gl.createFramebuffer();
      try {
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
        gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, cv);
        gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
        gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
        if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new SvgRasterError('unavailable', 'The SVG raster could not be read back.');
        // texImage2D from a canvas without UNPACK_FLIP_Y puts its top row at
        // texture row 0, which readPixels returns first: top-down.
        const out = new Uint8Array(w * h * 4);
        gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, out);
        return out;
      } finally {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.deleteFramebuffer(framebuffer);
        gl.bindTexture(gl.TEXTURE_2D, null);
        gl.deleteTexture(texture);
      }
    } finally {
      // As raster.ts releases its cache's canvases.
      cv.width = 0;
      cv.height = 0;
    }
  }
}

/** SVG bytes decoded as today's import decodes them: an `<img>` of an
 *  `image/svg+xml` blob (prepareAsset). */
export async function decodeSvg(bytes: Uint8Array): Promise<HTMLImageElement> {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: 'image/svg+xml' }));
  try {
    const img = new Image();
    img.src = url;
    await img.decode();
    return img;
  } catch (error) {
    throw new SvgRasterError('decode', `Chromium could not decode this SVG (${error instanceof Error ? error.message : String(error)})`);
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Lowercase hex SHA-256. */
export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as BufferSource));
  let out = '';
  for (const b of digest) out += b.toString(16).padStart(2, '0');
  return out;
}

const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
]);

/** Lowercase hex SHA-256 on the calling thread (FIPS 180-4), for a
 *  synchronous snapshot that meets an SVG not hashed yet; equal to
 *  `sha256Hex`. */
export function sha256HexSync(bytes: Uint8Array): string {
  const bitLength = bytes.length * 8;
  const padded = new Uint8Array(((bytes.length + 9 + 63) >> 6) << 6);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  const view = new DataView(padded.buffer);
  view.setUint32(padded.length - 8, Math.floor(bitLength / 0x100000000));
  view.setUint32(padded.length - 4, bitLength >>> 0);
  const h = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
  const w = new Uint32Array(64);
  const rotr = (x: number, n: number) => (x >>> n) | (x << (32 - n));
  for (let block = 0; block < padded.length; block += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(block + i * 4);
    for (let i = 16; i < 64; i++) {
      const a = w[i - 15]!, b = w[i - 2]!;
      w[i] = (w[i - 16]! + (rotr(a, 7) ^ rotr(a, 18) ^ (a >>> 3)) + w[i - 7]! + (rotr(b, 17) ^ rotr(b, 19) ^ (b >>> 10))) >>> 0;
    }
    let [a, b, c, d, e, f, g, hh] = h as unknown as [number, number, number, number, number, number, number, number];
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + SHA256_K[i]! + w[i]!) >>> 0;
      const t2 = ((rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) >>> 0;
      hh = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    h[0] = (h[0]! + a) >>> 0; h[1] = (h[1]! + b) >>> 0; h[2] = (h[2]! + c) >>> 0; h[3] = (h[3]! + d) >>> 0;
    h[4] = (h[4]! + e) >>> 0; h[5] = (h[5]! + f) >>> 0; h[6] = (h[6]! + g) >>> 0; h[7] = (h[7]! + hh) >>> 0;
  }
  let out = '';
  for (const word of h) out += word.toString(16).padStart(8, '0');
  return out;
}

/** The bytes behind a `blob:` URL of this window, on the calling thread: a
 *  synchronous XMLHttpRequest is the only synchronous read a window has. */
export function readBytesSync(url: string): Uint8Array {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', url, false);
  // Bytes as code units 0..255 (a synchronous request has no arraybuffer).
  xhr.overrideMimeType('text/plain; charset=x-user-defined');
  xhr.send();
  if (xhr.status !== 200 && xhr.status !== 0) throw new SvgRasterError('missing', `The SVG source could not be read (${xhr.status}).`);
  const text = xhr.responseText;
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) out[i] = text.charCodeAt(i) & 0xff;
  return out;
}

/** What the module needs of a live asset (`PM.assets.get`). */
export interface SvgAsset {
  el?: CanvasImageSource | null;
  url?: string;
  format?: string;
  kind?: string;
}

export type SvgRastersOptions = {
  /** The window's live asset (`PM.assets.get(id)`). */
  asset: (id: string) => SvgAsset | null | undefined;
  /** Every live asset (`PM.assets.map.values()`), for `prime`. */
  assets?: () => Iterable<SvgAsset>;
  /** The bytes the asset's `<img>` decoded (default: its object URL). */
  bytes?: (asset: SvgAsset) => Promise<Uint8Array>;
  /** The same bytes on the calling thread (default: `readBytesSync` of
   *  its object URL), for a synchronous snapshot that meets an asset not
   *  hashed yet (one drawn in the same task as its import or replace). */
  bytesSync?: (asset: SvgAsset) => Uint8Array;
  canvas?: SvgRasterCanvas;
  log?: (level: 'info' | 'warn', message: string) => void;
};

/** A session to post bands on (`DocumentSession.post`). */
export interface SvgPostTarget {
  post(event: Record<string, unknown> & { type: string }): boolean;
}

const keyText = (k: SvgRasterKey) => `${k.content}:${k.width}x${k.height}`;

function isSvg(asset: SvgAsset): boolean {
  return asset.format === 'svg' && (asset.kind === undefined || asset.kind === 'image');
}

export class SvgRasters {
  /** Content hashes of the bytes each asset's `<img>` decoded. */
  private readonly contents = new WeakMap<object, { url: string | undefined; content: Promise<string>; value?: string }>();
  private readonly inFlight = new Map<string, Promise<void>>();
  private readonly canvas: SvgRasterCanvas;
  private active = 0;
  private readonly queue: (() => void)[] = [];
  /** Counts (`hashedSync`: hashes a synchronous snapshot took on the
   *  calling thread), and the slowest viewer ask answered (ms from the ask
   *  to its last band). */
  readonly stats = { drawn: 0, posted: 0, produced: 0, failed: 0, stale: 0, hashedSync: 0, slowestMs: 0 };
  /** The latest viewer asks, oldest first (at most `MAX_ASK_LOG`): which
   *  buckets the engine's viewer frames wanted, for comparison with the
   *  buckets today's viewer rasterizes. */
  readonly asks: { asset: string; width: number; height: number }[] = [];

  constructor(private readonly options: SvgRastersOptions) {
    this.canvas = options.canvas ?? new SvgRasterCanvas();
  }

  private async readBytes(asset: SvgAsset): Promise<Uint8Array> {
    if (this.options.bytes) return this.options.bytes(asset);
    if (!asset.url) throw new SvgRasterError('missing', 'The SVG asset has no source in this window.');
    const response = await fetch(asset.url);
    return new Uint8Array(await response.arrayBuffer());
  }

  /** The content hash of the bytes `asset`'s `<img>` decoded (cached per
   *  asset object and source URL: a replaced or relinked asset is a new
   *  object or URL). */
  content(asset: SvgAsset): Promise<string> {
    const known = this.contents.get(asset as object);
    if (known && known.url === asset.url) return known.content;
    const entry: { url: string | undefined; content: Promise<string>; value?: string } = { url: asset.url, content: Promise.resolve('') };
    entry.content = this.readBytes(asset).then(sha256Hex).then((value) => {
      entry.value = value;
      return value;
    });
    // A failed read is not remembered.
    entry.content.catch(() => {
      if (this.contents.get(asset as object) === entry) this.contents.delete(asset as object);
    });
    this.contents.set(asset as object, entry);
    return entry.content;
  }

  /** The content hash when it is already known (synchronous callers). */
  knownContent(asset: SvgAsset): string | null {
    const known = this.contents.get(asset as object);
    return known && known.url === asset.url ? known.value ?? null : null;
  }

  /** The content hash on the calling thread: known, or the bytes read and
   *  hashed now (and remembered, as `content` remembers). */
  contentSync(asset: SvgAsset): string | null {
    const known = this.knownContent(asset);
    if (known !== null) return known;
    let bytes: Uint8Array;
    try {
      if (this.options.bytesSync) bytes = this.options.bytesSync(asset);
      else if (asset.url) bytes = readBytesSync(asset.url);
      else return null;
    } catch {
      return null;
    }
    const value = sha256HexSync(bytes);
    this.contents.set(asset as object, { url: asset.url, content: Promise.resolve(value), value });
    this.stats.hashedSync++;
    return value;
  }

  /** Hashes every live SVG asset ahead, so a synchronous snapshot can
   *  check one (the connection calls it on `assets` changes). */
  prime(): void {
    for (const asset of this.options.assets?.() ?? []) {
      if (isSvg(asset) && asset.el) void this.content(asset).catch(() => {});
    }
  }

  private checked(request: SvgRasterRequest, content: string | null): CanvasImageSource {
    const asset = this.options.asset(request.asset);
    if (!asset || !asset.el) throw new SvgRasterError('missing', `This window has no decoded SVG for asset ${request.asset}.`);
    if (content === null) throw new SvgRasterError('stale', `The SVG of asset ${request.asset} is not hashed yet in this window.`);
    if (content !== request.key.content) throw new SvgRasterError('stale', `The SVG of asset ${request.asset} in this window (${content.slice(0, 12)}) is not the engine's (${request.key.content.slice(0, 12)}): another version of the file.`);
    return asset.el;
  }

  private validate(key: SvgRasterKey): void {
    if (!(Number.isInteger(key.width) && Number.isInteger(key.height) && key.width >= 1 && key.height >= 1 && key.width <= MAX_SVG_EDGE && key.height <= MAX_SVG_EDGE)) {
      throw new SvgRasterError('size', `A ${key.width}×${key.height} SVG raster is outside 1..${MAX_SVG_EDGE}.`);
    }
  }

  /** The raster of `request`, asynchronously (the asset's bytes hashed
   *  first if they are not yet). */
  async draw(request: SvgRasterRequest): Promise<Uint8Array> {
    this.validate(request.key);
    const asset = this.options.asset(request.asset);
    const content = asset ? await this.content(asset) : null;
    const data = this.canvas.draw(this.checked(request, content), request.key.width, request.key.height);
    this.stats.drawn++;
    return data;
  }

  /** The raster of `request` on this thread (snapshots): with the content
   *  hash `prime` took ahead, or taken now (`contentSync`). */
  drawSync(request: SvgRasterRequest): Uint8Array {
    this.validate(request.key);
    const asset = this.options.asset(request.asset);
    const data = this.canvas.draw(this.checked(request, asset ? this.contentSync(asset) : null), request.key.width, request.key.height);
    this.stats.drawn++;
    return data;
  }

  /** Posts a raster in row bands on `target`. */
  post(target: SvgPostTarget, key: SvgRasterKey, data: Uint8Array): boolean {
    const rowBytes = key.width * 4;
    if (data.byteLength !== rowBytes * key.height) throw new SvgRasterError('size', 'SVG raster pixels do not match their size.');
    const rowsPerBand = Math.max(1, Math.floor(MAX_SVG_BAND_BYTES / rowBytes));
    for (let row = 0; row < key.height; row += rowsPerBand) {
      const rows = Math.min(rowsPerBand, key.height - row);
      if (!target.post({ type: 'compat', compat: 'svgPixels', key, row, rows, data: data.slice(row * rowBytes, (row + rows) * rowBytes) })) return false;
    }
    this.stats.posted++;
    return true;
  }

  private fail(target: SvgPostTarget, key: SvgRasterKey, error: unknown): void {
    const reason: SvgFailureReason = error instanceof SvgRasterError ? error.reason : 'unavailable';
    const message = (error instanceof Error ? error.message : String(error)).slice(0, 1024);
    this.stats.failed++;
    if (reason === 'stale') this.stats.stale++;
    this.options.log?.('warn', `engine: an SVG raster ${key.width}×${key.height} was not drawn (${reason}: ${message})`);
    target.post({ type: 'compat', compat: 'svgFailed', key, reason, message });
  }

  private async slot<T>(fn: () => Promise<T>): Promise<T> {
    if (this.active >= MAX_CONCURRENT) await new Promise<void>((resolve) => this.queue.push(resolve));
    this.active++;
    try {
      return await fn();
    } finally {
      this.active--;
      this.queue.shift()?.();
    }
  }

  /** The engine's asks for a viewer session: drawn and posted, or failed,
   *  once per key in flight. */
  answer(target: SvgPostTarget, requests: readonly SvgRasterRequest[]): Promise<void> {
    const all: Promise<void>[] = [];
    for (const request of requests) {
      this.asks.push({ asset: request.asset, width: request.key.width, height: request.key.height });
      if (this.asks.length > MAX_ASK_LOG) this.asks.splice(0, this.asks.length - MAX_ASK_LOG);
      const id = keyText(request.key);
      let running = this.inFlight.get(id);
      if (!running) {
        const asked = performance.now();
        running = this.slot(async () => {
          try {
            this.post(target, request.key, await this.draw(request));
            this.stats.slowestMs = Math.max(this.stats.slowestMs, performance.now() - asked);
          } catch (error) {
            this.fail(target, request.key, error);
          }
        }).finally(() => this.inFlight.delete(id));
        this.inFlight.set(id, running);
      }
      all.push(running);
    }
    return Promise.all(all).then(() => {});
  }

  /** A job engine frame's raster (main's `compatFrame` ask). */
  async produce(request: CompatFrameRequest): Promise<{ key: string; width: number; height: number; format: 'rgba8Premultiplied'; origin: [number, number]; scale: number; data: Uint8Array }> {
    const svg = request.svg;
    if (!svg) throw new SvgRasterError('missing', 'The ask names no SVG raster.');
    const data = await this.slot(() => this.draw(svg));
    this.stats.produced++;
    return { key: keyText(svg.key), width: svg.key.width, height: svg.key.height, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1, data };
  }
}
