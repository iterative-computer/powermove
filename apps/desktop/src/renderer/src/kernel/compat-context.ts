/*
 * `render.gl.context`, the compatibility WebGL2 context (a deprecated
 * compatibility member, owner amendment 23; plan section 20 "render.gl.context";
 * docs/engine/extensions-compat.md "The compatibility context").
 *
 * Today it is the compositor's WebGL2 context on the canvas a viewer handed
 * to `render.gl.init` (legacy/gl/compositor.ts `GL.init`: alpha false,
 * premultiplied, preserveDrawingBuffer, no antialiasing). Extensions see:
 *
 * 1. null until `init`, then the context (`if (!context) init(canvas)` in
 *    viewers frozen before the engine);
 * 2. `context.canvas`, the bound canvas (frozen viewers find their stage
 *    from it after a reload);
 * 3. the default framebuffer holding the frame the viewer shows: each frame
 *    the compositor clears it to opaque black and draws the composite at the
 *    canvas's size (`GL.render` "present"), so `readPixels` there reads the
 *    finished composite;
 * 4. drawing into the default framebuffer shows in the viewer on top of the
 *    composite until the next frame replaces it. WebM exports (the shipped
 *    exporter's `exportWebCodecs` and `exportRecorder`) render each frame
 *    with `GL.render` into that canvas (`renderInto`), which emits
 *    `preview:presented` → `frame:rendered`, then encode the canvas: what a
 *    handler draws there is in the exported frame. Every other export, and
 *    captures, read their own pass (`GL.renderToPixels`) and never include
 *    it;
 * 6. `context.canvas` holds the composite and any drawing over it
 *    (`preserveDrawingBuffer`): `drawImage(context.canvas)`, `toDataURL()`,
 *    `toBlob()` and texture uploads from it read them;
 * 5. its own textures, framebuffers and programs, as in any WebGL2 context.
 *
 * With "Use New Engine" on there is no compositor context. The engine
 * presents its frames over the bound canvas (engine-connection/
 * canvas-binding.ts, or the built-in viewer's own presentation), and an
 * extension that asks for `render.gl.context` gets a WebGL2 context on an
 * offscreen canvas with today's attributes, at the bound canvas's size:
 *
 * - (1) and (2) hold: null before `init`; `canvas` is the bound canvas (an
 *   own property of the context; the offscreen canvas is `compatCanvas`).
 * - (3) Reading back: before the first operation that reads or draws the
 *   default framebuffer after an engine frame was presented, the frame on
 *   screen (the presentation canvas, the region under the bound canvas) is
 *   uploaded into the default framebuffer over opaque black, so
 *   `readPixels`, `copyTex*Image2D` and `blitFramebuffer` from it read the
 *   composite, and drawing lands on top of it, as today.
 * - (4) Drawing: after the extension draws into the default framebuffer
 *   (clear, draw calls, blits), the offscreen canvas is composited as a
 *   layer of the viewer's presentation stack, right above the engine's
 *   canvas and under the viewer's overlay, at the bound canvas's place,
 *   until the next engine frame is presented (the frame that replaces the
 *   drawing today). WebM exports: each frame the job engine renders comes
 *   to this window before it is encoded ([`exportFrame`]); the default
 *   framebuffer holds it at the export's size, `frame:rendered` is
 *   emitted for it, and when a handler drew, the frame encoded is what
 *   the default framebuffer then holds (else the engine's own). Other
 *   exports and captures never include it, as today.
 * - (5) holds: it is an ordinary WebGL2 context.
 * - (6) The bound canvas gets today's pixels: each presented frame is
 *   copied into it (a 2D context of its own, opaque), then any drawing
 *   over it, so reads of `context.canvas` see what today's canvas held.
 *   It rests hidden under the presentation, so nothing on screen changes.
 *
 * The context stays the same object while the same canvas is bound; a new
 * `init` with another canvas makes a new one, as `GL.init` did.
 */
type LegacyPM = Record<string, any>;

/** Today's compositor attributes (`GL.init`). */
export const COMPAT_ATTRIBUTES: WebGLContextAttributes = {
  alpha: false, antialias: false, premultipliedAlpha: true,
  preserveDrawingBuffer: true, powerPreference: 'high-performance', desynchronized: true
};

/** Methods that draw into the bound draw framebuffer. */
const DRAWS = [
  'clear', 'clearBufferfv', 'clearBufferiv', 'clearBufferuiv', 'clearBufferfi',
  'drawArrays', 'drawElements', 'drawArraysInstanced', 'drawElementsInstanced', 'drawRangeElements'
] as const;
/** Methods that read the bound read framebuffer. */
const READS = ['readPixels', 'copyTexImage2D', 'copyTexSubImage2D', 'copyTexSubImage3D'] as const;

/** What the host needs of its surroundings (the app; fakes in tests). */
export type CompatContextEnv = {
  /** The canvas `render.gl.init` bound with the engine on, or null. */
  bound(): HTMLCanvasElement | null;
  /** Whether the engine draws this window's frames (else the compositor's context is the answer). */
  engine(): boolean;
  /** The compositor's own context (the flag off). */
  legacy(): WebGL2RenderingContext | null;
  /** Reserve a deferred compositor's WebGL context before mirroring can
   * claim its canvas as Canvas2D. Only an explicit context read pays this. */
  beforeContext?(bound: HTMLCanvasElement): void;
  /** Makes the offscreen canvas. */
  makeCanvas?(width: number, height: number): OffscreenCanvas | HTMLCanvasElement;
  /** The engine's canvas presenting over `bound`, when it shows a frame. */
  presentation?(bound: HTMLCanvasElement): HTMLCanvasElement | null;
  /** Runs `fn` after the current task's drawing (a microtask). */
  defer?(fn: () => void): void;
};

type CompatState = {
  bound: HTMLCanvasElement;
  canvas: OffscreenCanvas | HTMLCanvasElement;
  gl: WebGL2RenderingContext;
  /** Unwrapped methods. */
  raw: Record<string, (...args: any[]) => any>;
  drawFb: WebGLFramebuffer | null;
  readFb: WebGLFramebuffer | null;
  /** An engine frame was presented after the last upload. */
  stale: boolean;
  /** The extension drew into the default framebuffer since the last frame. */
  dirty: boolean;
  flushQueued: boolean;
  layer: HTMLCanvasElement | null;
  upload: { tex: WebGLTexture; fbo: WebGLFramebuffer } | null;
  lost: boolean;
  /** An export frame the default framebuffer holds instead of the frame on screen ([`exportFrame`]). */
  exporting: ImageData | null;
  /** Draw calls into the default framebuffer, and how many of them are shown. */
  drawn: number;
  shown: number;
  /** The bound canvas's 2D context mirroring today's canvas pixels (`context.canvas` reads). */
  mirror: CanvasRenderingContext2D | null | undefined;
  /** Preserve a presented WebGPU canvas before Chromium clears its swapchain. */
  frame: OffscreenCanvas | HTMLCanvasElement | null;
  copies: Map<string, OffscreenCanvas | HTMLCanvasElement>;
  pendingFrame: OffscreenCanvas | HTMLCanvasElement | null;
};

/** Counters for tests and diagnostics. */
export type CompatContextStats = { made: number; uploads: number; shown: number; hidden: number; mirrored: number; exportFrames: number; recreated: string | null; copiesAtRecreate: number; selectedAtRecreate: boolean };

function defaultCanvas(width: number, height: number): OffscreenCanvas | HTMLCanvasElement {
  if (typeof OffscreenCanvas === 'function') return new OffscreenCanvas(width, height);
  const canvas = document.createElement('canvas');
  canvas.width = width; canvas.height = height;
  return canvas;
}

const frameKey = (tag: { seq: number; revision: number; viewSeq: number }) => `${tag.revision}:${tag.viewSeq}:${tag.seq}`;

/** The engine's canvas over `bound`: the presentation in the bound canvas's stage, when it shows a frame. */
export function presentationCanvas(bound: HTMLCanvasElement): HTMLCanvasElement | null {
  const scopes = [bound.closest('[data-engine-stage]'), bound.parentElement].filter(Boolean) as Element[];
  for (const scope of scopes) {
    for (const canvas of Array.from(scope.querySelectorAll('canvas#engine-present')) as HTMLCanvasElement[]) {
      if (canvas.style.visibility === 'visible' && canvas.width > 0 && canvas.height > 0) return canvas;
    }
  }
  return null;
}

export class CompatContextHost {
  private state: CompatState | null = null;
  readonly stats: CompatContextStats = { made: 0, uploads: 0, shown: 0, hidden: 0, mirrored: 0, exportFrames: 0, recreated: null, copiesAtRecreate: 0, selectedAtRecreate: false };

  constructor(private readonly env: CompatContextEnv) {}

  /** `render.gl.context`. */
  context(): WebGL2RenderingContext | null {
    if (!this.env.engine()) return this.env.legacy();
    const bound = this.env.bound();
    if (!bound) return null;
    let state = this.state;
    if (!state || state.bound !== bound || state.lost) {
      const previous = state;
      if (previous) {
        this.stats.recreated = previous.lost ? 'lost' : 'bound';
        this.stats.copiesAtRecreate = previous.copies.size;
        this.stats.selectedAtRecreate = !!previous.pendingFrame;
      }
      this.drop();
      this.env.beforeContext?.(bound);
      state = this.make(bound);
      if (!state) return null;
      this.state = state;
      // A viewer can replace its bound canvas while paused, after the last
      // native frame. Keep the selected completed surface: the WebGPU
      // swapchain may already be clear and there may be no next frame.
      if (previous?.pendingFrame) {
        state.copies = previous.copies;
        state.pendingFrame = previous.pendingFrame;
        this.presented();
      }
    }
    this.fit(state);
    return state.gl;
  }

  /** The offscreen canvas behind the compatibility context (null before one was made). */
  get compatCanvas(): OffscreenCanvas | HTMLCanvasElement | null {
    return this.state?.canvas ?? null;
  }

  /** The layer showing the context's drawing, while it shows. */
  get shownLayer(): HTMLCanvasElement | null {
    const layer = this.state?.layer;
    return layer && layer.style.visibility === 'visible' ? layer : null;
  }

  /** Copy a completed engine surface before the presenter releases its pool
   * slot. WebGPU canvases clear after compositing, so reading one in the
   * coalesced frame event (the next rAF) can otherwise read black. Only a
   * requested compatibility context pays for this preservation. */
  captureFrame(tag: { seq: number; revision: number; viewSeq: number; colorSpace?: string }, source: VideoFrame): void {
    const state = this.state;
    if (!state) return;
    const canvas = (this.env.makeCanvas ?? defaultCanvas)(source.displayWidth, source.displayHeight);
    const c2 = canvas.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
    if (!c2) return;
    c2.drawImage(source, 0, 0);
    // Electron's shared texture import tags these RGB surfaces as sRGB even
    // when the descriptor says the engine encoded Display P3. The presenter
    // writes those values to its P3 canvas. Our legacy GL context is sRGB:
    // reinterpret the imported values before converting to that space.
    // (TypeScript's DOM library predates the 'smpte432' primaries value.)
    const primaries: string | null | undefined = source.colorSpace?.primaries;
    if (tag.colorSpace === 'displayP3' && primaries !== 'smpte432') {
      const pixels = c2.getImageData(0, 0, canvas.width, canvas.height);
      c2.putImageData(new ImageData(pixels.data, canvas.width, canvas.height, { colorSpace: 'display-p3' }), 0, 0);
    }
    state.copies.set(frameKey(tag), canvas);
    while (state.copies.size > 8) state.copies.delete(state.copies.keys().next().value!);
  }

  /** Select the surface for the exact frame whose public event follows. */
  selectFrame(tag: { seq: number; revision: number; viewSeq: number }): void {
    if (this.state) this.state.pendingFrame = this.state.copies.get(frameKey(tag)) ?? null;
  }

  /**
   * An engine frame was presented (today's `preview:presented`, emitted
   * before `frame:rendered`): the default framebuffer is to hold it again,
   * and the drawing it replaces stops showing.
   */
  presented(): void {
    const state = this.state;
    if (!state) return;
    state.stale = true;
    state.dirty = false;
    this.hideLayer(state);
    // `context.canvas` is the bound canvas; today it held the composite
    // (`preserveDrawingBuffer`), so `drawImage(context.canvas)`,
    // `toDataURL()`, `toBlob()` and texture uploads from it read the frame.
    // The bound canvas rests hidden under the engine's presentation: it
    // gets the frame on screen, then any drawing over it ([`showLayer`]).
    const source = (this.env.presentation ?? presentationCanvas)(state.bound);
    if (source) {
      const from = sourceRect(source, state.bound);
      const pixels = state.pendingFrame ?? source;
      if (from) {
        let frame = state.frame;
        const width = state.bound.width, height = state.bound.height;
        if (!frame) frame = state.frame = (this.env.makeCanvas ?? defaultCanvas)(width, height);
        if (frame.width !== width) frame.width = width;
        if (frame.height !== height) frame.height = height;
        const c2 = frame.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
        if (c2) {
          c2.globalCompositeOperation = 'copy'; c2.fillStyle = '#000'; c2.fillRect(0, 0, width, height);
          c2.globalCompositeOperation = 'source-over';
          const same = Math.abs(from.sw - width) <= 1 && Math.abs(from.sh - height) <= 1;
          c2.imageSmoothingEnabled = !same;
          c2.drawImage(pixels as CanvasImageSource, same ? Math.round(from.sx) : from.sx, same ? Math.round(from.sy) : from.sy,
            same ? width : from.sw, same ? height : from.sh, 0, 0, width, height);
          this.mirror(state, frame as CanvasImageSource);
        }
      }
    }
  }

  /**
   * Runs `emit` (today's `preview:presented` → `frame:rendered` of an
   * export frame) with the default framebuffer holding `frame` at its
   * size, as today's `renderInto` left the compositor's canvas for the
   * handlers, and returns what the default framebuffer holds after them
   * when any of them drew into it (RGBA8, top-down, opaque), else null:
   * the frame is the engine's. Only WebM exports call it (the exports
   * whose frames today's exporter read from that canvas).
   */
  exportFrame(frame: { width: number; height: number; data: Uint8Array }, emit: () => void): Uint8Array | null {
    const state = this.state;
    if (!state || state.lost || !this.env.engine()) {
      emit();
      return null;
    }
    const { width, height } = frame;
    if (!(width >= 1 && height >= 1) || frame.data.byteLength !== width * height * 4) throw new Error(`The export frame is not ${width}×${height} RGBA.`);
    const pixels = new Uint8ClampedArray(width * height * 4);
    pixels.set(frame.data);
    const image = new ImageData(pixels, width, height);
    const before = { width: state.canvas.width, height: state.canvas.height };
    state.exporting = image;
    state.canvas.width = width;
    state.canvas.height = height;
    state.stale = true;
    state.dirty = false;
    try {
      emit();
      if (!state.dirty || state.lost) return null;
      const gl = state.gl;
      const raw = state.raw;
      const rows = new Uint8Array(width * height * 4);
      raw.bindFramebuffer!(gl.READ_FRAMEBUFFER, null);
      const pack = { buffer: gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING), alignment: gl.getParameter(gl.PACK_ALIGNMENT), rowLength: gl.getParameter(gl.PACK_ROW_LENGTH), skipPixels: gl.getParameter(gl.PACK_SKIP_PIXELS), skipRows: gl.getParameter(gl.PACK_SKIP_ROWS) };
      try {
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
        gl.pixelStorei(gl.PACK_ALIGNMENT, 4);
        gl.pixelStorei(gl.PACK_ROW_LENGTH, 0);
        gl.pixelStorei(gl.PACK_SKIP_PIXELS, 0);
        gl.pixelStorei(gl.PACK_SKIP_ROWS, 0);
        raw.readPixels!(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, rows);
      } finally {
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, pack.buffer);
        gl.pixelStorei(gl.PACK_ALIGNMENT, pack.alignment);
        gl.pixelStorei(gl.PACK_ROW_LENGTH, pack.rowLength);
        gl.pixelStorei(gl.PACK_SKIP_PIXELS, pack.skipPixels);
        gl.pixelStorei(gl.PACK_SKIP_ROWS, pack.skipRows);
        raw.bindFramebuffer!(gl.READ_FRAMEBUFFER, state.readFb);
      }
      // Bottom row first in GL: flip to top-down; the canvas is opaque.
      const out = new Uint8Array(rows.length);
      const stride = width * 4;
      for (let y = 0; y < height; y++) out.set(rows.subarray((height - 1 - y) * stride, (height - y) * stride), y * stride);
      for (let i = 3; i < out.length; i += 4) out[i] = 255;
      this.stats.exportFrames++;
      return out;
    } finally {
      state.exporting = null;
      state.canvas.width = before.width;
      state.canvas.height = before.height;
      state.stale = true;
      state.dirty = false;
    }
  }

  /** Whether an extension holds the compatibility context (export frames then go through it). */
  get active(): boolean {
    return !!this.state && !this.state.lost;
  }

  /** A requested context still needs completed engine pixels while its
   * WebGL context is lost, so a paused recovery can read the current frame. */
  get capturesFrames(): boolean {
    return !!this.state;
  }

  /**
   * Copies `source` (the frame on screen, or the context's drawing over
   * it) into the bound canvas's own pixels, opaque as today's canvas
   * (`alpha: false`). A bound canvas that already has another kind of
   * context is left alone.
   */
  private mirror(state: CompatState, source: CanvasImageSource, from?: { sx: number; sy: number; sw: number; sh: number }): void {
    if (state.mirror === undefined) {
      try {
        state.mirror = state.bound.getContext('2d', { alpha: false }) as CanvasRenderingContext2D | null;
      } catch {
        state.mirror = null;
      }
    }
    const width = state.bound.width, height = state.bound.height;
    // A bound canvas that already has today's compositor context (kept
    // beside the engine while the legacy compositor still runs): the
    // pixels go into its default framebuffer instead, through a 2D copy at
    // the bound canvas's size.
    const c2 = state.mirror ?? this.scratchMirror(width, height);
    if (!c2) return;
    try {
      c2.setTransform(1, 0, 0, 1, 0, 0);
      c2.globalCompositeOperation = 'copy';
      if (from) {
        const same = Math.abs(from.sw - width) <= 1 && Math.abs(from.sh - height) <= 1;
        c2.imageSmoothingEnabled = !same;
        c2.fillStyle = '#000';
        c2.fillRect(0, 0, width, height);
        c2.globalCompositeOperation = 'source-over';
        if (same) c2.drawImage(source, Math.round(from.sx), Math.round(from.sy), width, height, 0, 0, width, height);
        else c2.drawImage(source, from.sx, from.sy, from.sw, from.sh, 0, 0, width, height);
      } else {
        c2.imageSmoothingEnabled = false;
        c2.drawImage(source, 0, 0, width, height);
      }
      if (!state.mirror && !this.uploadToBoundGl(state, c2.canvas as OffscreenCanvas | HTMLCanvasElement)) return;
      this.stats.mirrored++;
    } catch {
      // A canvas that cannot be read (lost device): today's would be black too.
    }
  }

  private mirrorCanvas: OffscreenCanvas | HTMLCanvasElement | null = null;

  private scratchMirror(width: number, height: number): CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null {
    try {
      let canvas = this.mirrorCanvas;
      if (!canvas) canvas = this.mirrorCanvas = (this.env.makeCanvas ?? defaultCanvas)(width, height);
      if (canvas.width !== width) canvas.width = width;
      if (canvas.height !== height) canvas.height = height;
      return canvas.getContext('2d', { alpha: false }) as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
    } catch {
      return null;
    }
  }

  /** Writes `image` into the default framebuffer of the bound canvas's own
   *  WebGL2 context (today's compositor's), leaving its state as it was. */
  private uploadToBoundGl(state: CompatState, image: OffscreenCanvas | HTMLCanvasElement): boolean {
    let gl: WebGL2RenderingContext | null = null;
    try {
      gl = state.bound.getContext('webgl2') as WebGL2RenderingContext | null;
    } catch {
      gl = null;
    }
    if (!gl || gl.isContextLost()) return false;
    const width = state.bound.width, height = state.bound.height;
    const saved = {
      read: gl.getParameter(gl.READ_FRAMEBUFFER_BINDING), draw: gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING),
      tex: gl.getParameter(gl.TEXTURE_BINDING_2D),
      unpackBuffer: gl.getParameter(gl.PIXEL_UNPACK_BUFFER_BINDING), flip: gl.getParameter(gl.UNPACK_FLIP_Y_WEBGL),
      premultiply: gl.getParameter(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL), conversion: gl.getParameter(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL),
      alignment: gl.getParameter(gl.UNPACK_ALIGNMENT), scissor: gl.isEnabled(gl.SCISSOR_TEST), discard: gl.isEnabled(gl.RASTERIZER_DISCARD)
    };
    const tex = gl.createTexture(), fbo = gl.createFramebuffer();
    try {
      // (on the active unit, whose binding is restored below)
      gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, null);
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.BROWSER_DEFAULT_WEBGL);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, image as TexImageSource);
      gl.disable(gl.SCISSOR_TEST);
      gl.disable(gl.RASTERIZER_DISCARD);
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, fbo);
      gl.framebufferTexture2D(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
      gl.blitFramebuffer(0, 0, width, height, 0, 0, width, height, gl.COLOR_BUFFER_BIT, gl.NEAREST);
      return true;
    } finally {
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, saved.read);
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, saved.draw);
      gl.deleteFramebuffer(fbo);
      gl.deleteTexture(tex);
      gl.bindTexture(gl.TEXTURE_2D, saved.tex);
      gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, saved.unpackBuffer);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, saved.flip);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, saved.premultiply);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, saved.conversion);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, saved.alignment);
      if (saved.scissor) gl.enable(gl.SCISSOR_TEST);
      if (saved.discard) gl.enable(gl.RASTERIZER_DISCARD);
    }
  }

  dispose(): void {
    this.drop();
  }

  private make(bound: HTMLCanvasElement): CompatState | null {
    const width = Math.max(1, bound.width | 0), height = Math.max(1, bound.height | 0);
    let canvas: OffscreenCanvas | HTMLCanvasElement;
    let gl: WebGL2RenderingContext | null = null;
    try {
      canvas = (this.env.makeCanvas ?? defaultCanvas)(width, height);
      gl = canvas.getContext('webgl2', COMPAT_ATTRIBUTES) as WebGL2RenderingContext | null;
    } catch {
      return null;
    }
    if (!gl) return null;
    const state: CompatState = {
      bound, canvas, gl, raw: {}, drawFb: null, readFb: null,
      stale: true, dirty: false, flushQueued: false, layer: null, upload: null, lost: false, exporting: null, mirror: undefined, frame: null, copies: new Map(), pendingFrame: null, drawn: 0, shown: 0
    };
    this.instrument(state);
    (canvas as any).addEventListener?.('webglcontextlost', (event: Event) => {
      event.preventDefault();
      state.lost = true;
      state.upload = null;
    });
    this.stats.made++;
    return state;
  }

  /** Follows the bound canvas's size (today `GL.resize` sized the compositor's canvas). */
  private fit(state: CompatState): void {
    // An export frame keeps the export's size until its handlers ran.
    if (state.exporting) return;
    const width = Math.max(1, state.bound.width | 0), height = Math.max(1, state.bound.height | 0);
    if (state.canvas.width !== width || state.canvas.height !== height) {
      state.canvas.width = width;
      state.canvas.height = height;
      state.stale = true;
    }
  }

  /** Wraps the context's framebuffer-touching methods (own properties: the context stays the real object). */
  private instrument(state: CompatState): void {
    const gl = state.gl as any;
    const raw = state.raw;
    const own = (name: string, value: unknown) => Object.defineProperty(gl, name, { configurable: true, writable: true, value });
    for (const name of ['bindFramebuffer', 'deleteFramebuffer', 'blitFramebuffer', ...DRAWS, ...READS]) {
      const fn = gl[name];
      if (typeof fn === 'function') raw[name] = fn.bind(gl);
    }
    // Reading `context.canvas` right after drawing (`drawImage(gl.canvas)`)
    // sees the drawing, as today's canvas held it at once.
    Object.defineProperty(gl, 'canvas', { configurable: true, get: () => { this.flush(); return state.bound; } });
    Object.defineProperty(gl, 'compatCanvas', { configurable: true, get: () => state.canvas });
    own('bindFramebuffer', (target: number, fb: WebGLFramebuffer | null) => {
      raw.bindFramebuffer!(target, fb);
      if (target === gl.FRAMEBUFFER) { state.drawFb = fb; state.readFb = fb; }
      else if (target === gl.DRAW_FRAMEBUFFER) state.drawFb = fb;
      else if (target === gl.READ_FRAMEBUFFER) state.readFb = fb;
    });
    own('deleteFramebuffer', (fb: WebGLFramebuffer | null) => {
      raw.deleteFramebuffer!(fb);
      if (fb && state.drawFb === fb) state.drawFb = null;
      if (fb && state.readFb === fb) state.readFb = null;
    });
    for (const name of DRAWS) {
      if (!raw[name]) continue;
      own(name, (...args: any[]) => {
        const target = state.drawFb === null;
        if (target) this.refresh(state);
        const result = raw[name]!(...args);
        if (target) this.drew(state);
        return result;
      });
    }
    for (const name of READS) {
      if (!raw[name]) continue;
      own(name, (...args: any[]) => {
        if (state.readFb === null) this.refresh(state);
        return raw[name]!(...args);
      });
    }
    if (raw.blitFramebuffer) {
      own('blitFramebuffer', (...args: any[]) => {
        const touches = state.drawFb === null || state.readFb === null;
        if (touches) this.refresh(state);
        const result = raw.blitFramebuffer!(...args);
        if (state.drawFb === null) this.drew(state);
        return result;
      });
    }
  }

  /**
   * Puts the frame on screen into the default framebuffer, if an engine
   * frame was presented since the last time: opaque black (today's clear),
   * then the presentation's pixels under the bound canvas at its size.
   * The extension's GL state is left as it was.
   */
  private refresh(state: CompatState): void {
    this.fit(state);
    if (!state.stale) return;
    state.stale = false;
    const gl = state.gl;
    const raw = state.raw;
    const width = state.canvas.width, height = state.canvas.height;
    const source = state.exporting ? null : (this.env.presentation ?? presentationCanvas)(state.bound);
    // The frame under the bound canvas, scaled to its backing store, over black.
    const scratch = this.scratch(width, height);
    const c2 = scratch?.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
    if (!scratch || !c2) return;
    c2.setTransform(1, 0, 0, 1, 0, 0);
    c2.globalCompositeOperation = 'copy';
    c2.fillStyle = '#000';
    c2.fillRect(0, 0, width, height);
    c2.globalCompositeOperation = 'source-over';
    // An export frame (opaque, at the export's size: today's `renderInto`
    // resized the compositor's canvas to it).
    if (state.exporting) c2.putImageData(state.exporting, 0, 0);
    if (!state.exporting && state.frame && state.frame.width === width && state.frame.height === height) {
      c2.drawImage(state.frame as CanvasImageSource, 0, 0);
    } else if (source) {
      const from = sourceRect(source, state.bound);
      if (from) {
        // At the same density (the usual case) the region is copied pixel
        // for pixel from the nearest whole-pixel origin; otherwise scaled.
        // (A CSS box of fractional size rounds its backing store: within a pixel is the same density.)
        const same = Math.abs(from.sw - width) <= 1 && Math.abs(from.sh - height) <= 1;
        if (same) Object.assign(from, { sx: Math.round(from.sx), sy: Math.round(from.sy), sw: width, sh: height });
        c2.imageSmoothingEnabled = !same;
        try {
          c2.drawImage(source, from.sx, from.sy, from.sw, from.sh, 0, 0, width, height);
        } catch {
          // A canvas that cannot be read (lost device): black, as a cleared frame.
        }
      }
    }
    const saved = {
      tex: gl.getParameter(gl.TEXTURE_BINDING_2D),
      unpackBuffer: gl.getParameter(gl.PIXEL_UNPACK_BUFFER_BINDING),
      flip: gl.getParameter(gl.UNPACK_FLIP_Y_WEBGL),
      premultiply: gl.getParameter(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL),
      conversion: gl.getParameter(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL),
      alignment: gl.getParameter(gl.UNPACK_ALIGNMENT),
      rowLength: gl.getParameter(gl.UNPACK_ROW_LENGTH),
      skipPixels: gl.getParameter(gl.UNPACK_SKIP_PIXELS),
      skipRows: gl.getParameter(gl.UNPACK_SKIP_ROWS),
      scissor: gl.isEnabled(gl.SCISSOR_TEST),
      discard: gl.isEnabled(gl.RASTERIZER_DISCARD)
    };
    try {
      if (!state.upload) {
        const tex = gl.createTexture(), fbo = gl.createFramebuffer();
        if (!tex || !fbo) return;
        state.upload = { tex, fbo };
      }
      const { tex, fbo } = state.upload;
      gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, null);
      gl.bindTexture(gl.TEXTURE_2D, tex);
      // Bottom row first, as the default framebuffer reads back.
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.BROWSER_DEFAULT_WEBGL);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
      gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0);
      gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 0);
      gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 0);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, scratch as TexImageSource);
      gl.disable(gl.SCISSOR_TEST);
      gl.disable(gl.RASTERIZER_DISCARD);
      raw.bindFramebuffer!(gl.READ_FRAMEBUFFER, fbo);
      gl.framebufferTexture2D(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
      raw.bindFramebuffer!(gl.DRAW_FRAMEBUFFER, null);
      raw.blitFramebuffer!(0, 0, width, height, 0, 0, width, height, gl.COLOR_BUFFER_BIT, gl.NEAREST);
      this.stats.uploads++;
    } finally {
      raw.bindFramebuffer!(gl.READ_FRAMEBUFFER, state.readFb);
      raw.bindFramebuffer!(gl.DRAW_FRAMEBUFFER, state.drawFb);
      gl.bindTexture(gl.TEXTURE_2D, saved.tex);
      gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, saved.unpackBuffer);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, saved.flip);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, saved.premultiply);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, saved.conversion);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, saved.alignment);
      gl.pixelStorei(gl.UNPACK_ROW_LENGTH, saved.rowLength);
      gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, saved.skipPixels);
      gl.pixelStorei(gl.UNPACK_SKIP_ROWS, saved.skipRows);
      if (saved.scissor) gl.enable(gl.SCISSOR_TEST);
      if (saved.discard) gl.enable(gl.RASTERIZER_DISCARD);
    }
  }

  private scratchCanvas: OffscreenCanvas | HTMLCanvasElement | null = null;

  private scratch(width: number, height: number): OffscreenCanvas | HTMLCanvasElement | null {
    try {
      let scratch = this.scratchCanvas;
      if (!scratch) scratch = this.scratchCanvas = (this.env.makeCanvas ?? defaultCanvas)(width, height);
      if (scratch.width !== width) scratch.width = width;
      if (scratch.height !== height) scratch.height = height;
      return scratch;
    } catch {
      return null;
    }
  }

  /** The extension drew into the default framebuffer: show it after this task. */
  private drew(state: CompatState): void {
    state.dirty = true;
    state.drawn++;
    // An export frame's drawing goes to the export, not the viewer.
    if (state.exporting) return;
    if (state.flushQueued) return;
    state.flushQueued = true;
    (this.env.defer ?? queueMicrotask)(() => {
      state.flushQueued = false;
      if (this.state === state) this.flush();
    });
  }

  /** Shows (and mirrors into the bound canvas) a drawing not shown yet. */
  flush(): void {
    const state = this.state;
    if (!state || !state.dirty || state.exporting || state.shown === state.drawn) return;
    state.shown = state.drawn;
    this.showLayer(state);
  }

  /** Composites the context's drawing over the engine's canvas, under the viewer's overlay. */
  private showLayer(state: CompatState): void {
    const bound = state.bound;
    const doc = bound.ownerDocument;
    if (!doc || !bound.isConnected) return;
    let layer = state.layer;
    if (!layer || layer.ownerDocument !== doc) {
      layer?.remove();
      layer = doc.createElement('canvas');
      layer.id = 'engine-compat-context';
      layer.setAttribute('aria-hidden', 'true');
      layer.style.cssText = 'position:absolute;pointer-events:none;visibility:hidden';
      state.layer = layer;
    }
    const above = (this.env.presentation ?? presentationCanvas)(bound) ?? bound;
    if (above.nextSibling !== layer) above.after(layer);
    const parent = (layer.offsetParent ?? layer.parentElement) as HTMLElement | null;
    const box = bound.getBoundingClientRect();
    const origin = parent?.getBoundingClientRect() ?? { left: 0, top: 0 };
    layer.style.left = `${box.left - origin.left + (parent?.scrollLeft ?? 0) - (parent?.clientLeft ?? 0)}px`;
    layer.style.top = `${box.top - origin.top + (parent?.scrollTop ?? 0) - (parent?.clientTop ?? 0)}px`;
    layer.style.width = `${box.width}px`;
    layer.style.height = `${box.height}px`;
    if (layer.width !== state.canvas.width) layer.width = state.canvas.width;
    if (layer.height !== state.canvas.height) layer.height = state.canvas.height;
    const c2 = layer.getContext('2d');
    if (!c2) return;
    c2.globalCompositeOperation = 'copy';
    c2.imageSmoothingEnabled = false;
    c2.drawImage(state.canvas as CanvasImageSource, 0, 0);
    layer.style.visibility = 'visible';
    this.stats.shown++;
    // Today the drawing was on the canvas itself: `context.canvas` reads it.
    this.mirror(state, state.canvas as CanvasImageSource);
  }

  private hideLayer(state: CompatState): void {
    const layer = state.layer;
    if (layer && layer.style.visibility === 'visible') {
      layer.style.visibility = 'hidden';
      this.stats.hidden++;
    }
  }

  private drop(): void {
    const state = this.state;
    if (!state) return;
    state.layer?.remove();
    state.layer = null;
    this.state = null;
  }
}

/** The region of `source` (the engine's canvas) on screen under `bound`, in `source`'s pixels. */
export function sourceRect(source: HTMLCanvasElement, bound: HTMLCanvasElement): { sx: number; sy: number; sw: number; sh: number } | null {
  const s = source.getBoundingClientRect();
  const b = bound.getBoundingClientRect();
  if (!(s.width > 0 && s.height > 0 && b.width > 0 && b.height > 0)) return null;
  const kx = source.width / s.width, ky = source.height / s.height;
  return { sx: (b.left - s.left) * kx, sy: (b.top - s.top) * ky, sw: b.width * kx, sh: b.height * ky };
}

/** `render.gl.context` for `PM` (the kernel's host, or today's compositor context). */
export function compatContext(PM: LegacyPM): WebGL2RenderingContext | null {
  const host = PM?.CompatContext as CompatContextHost | undefined;
  if (host) return host.context();
  if (PM?.EngineRender && PM?.EngineCanvas) return null;
  return (PM?.GL?.gl ?? null) as WebGL2RenderingContext | null;
}

/** The host for `PM`: the engine's binding, with today's compositor context when the engine is off. */
export function createCompatContextHost(PM: LegacyPM): CompatContextHost {
  return new CompatContextHost({
    bound: () => (PM?.EngineCanvas?.bound ?? null) as HTMLCanvasElement | null,
    engine: () => !!PM?.EngineRender && !!PM?.EngineCanvas,
    legacy: () => (PM?.GL?.gl ?? null) as WebGL2RenderingContext | null,
    beforeContext: (bound) => {
      if (PM?.GL?.canvas === bound) void PM.GL.gl;
    }
  });
}
