// WebGL2 call recorder. Captures every state-changing call three.js makes on the
// output context, with buffer, uniform, and texture payloads, so the Rust replayer can
// re-execute the exact command stream through the GLSL→SPIR-V→naga→wgpu pipeline.
//
// Image uploads are snapshotted through a second context with the same unpack state,
// so the trace holds the exact texels WebGL received (flip and premultiply applied).

type Arg = number | boolean | string | null | { f: string } | { o: number } | { b: number; n: number; t: string } | { img: number; w: number; h: number } | Arg[];

const SKIP = new Set([
  'getParameter', 'getExtension', 'getSupportedExtensions', 'getContextAttributes', 'isContextLost', 'getError',
  'getShaderPrecisionFormat', 'getProgramParameter', 'getShaderParameter', 'getProgramInfoLog', 'getShaderInfoLog',
  'getShaderSource', 'checkFramebufferStatus', 'getBufferSubData', 'getTexParameter', 'getFramebufferAttachmentParameter',
  'getRenderbufferParameter', 'getVertexAttrib', 'getVertexAttribOffset', 'getUniform', 'getIndexedParameter',
  'getInternalformatParameter', 'getQueryParameter', 'getSyncParameter', 'getActiveUniformBlockParameter',
  'getActiveUniforms', 'getAttachedShaders', 'getTranslatedShaderSource', 'isBuffer', 'isTexture', 'isProgram', 'isShader',
  'isFramebuffer', 'isRenderbuffer', 'isEnabled', 'isVertexArray', 'isSampler', 'isQuery', 'isSync', 'flush', 'finish',
  'readPixels', 'makeXRCompatible', 'getActiveUniformBlockName', 'getFragDataLocation', 'getUniformIndices',
]);
const RETURNS_OBJECT = /^create|^fenceSync$/;
const TYPED: [any, string][] = [
  [Float32Array, 'f32'], [Uint8Array, 'u8'], [Uint8ClampedArray, 'u8'], [Uint16Array, 'u16'], [Uint32Array, 'u32'],
  [Int8Array, 'i8'], [Int16Array, 'i16'], [Int32Array, 'i32'], [Float64Array, 'f64'],
];

export class GlRecorder {
  gl: WebGL2RenderingContext | null = null;
  armed = false;
  on = false;
  calls: any[] = [];
  private blobs: Uint8Array[] = [];
  private blobLength = 0;
  private ids = new WeakMap<object, number>();
  private nextId = 1;
  private unpack = { flipY: false, premultiply: false, colorspace: 0x9244 /* BROWSER_DEFAULT_WEBGL */ };
  private snap: WebGL2RenderingContext | null = null;
  private lastSize = '';

  arm() { this.armed = true; this.calls = []; this.blobs = []; this.blobLength = 0; this.nextId = 1; this.ids = new WeakMap(); this.lastSize = ''; this.unpack = { flipY: false, premultiply: false, colorspace: 0x9244 }; }
  mark(name: string, extra: Record<string, unknown> = {}) { this.calls.push(['__mark', name, extra]); }

  take(): { calls: any[]; blob: Uint8Array } {
    const blob = new Uint8Array(this.blobLength);
    let offset = 0;
    for (const b of this.blobs) { blob.set(b, offset); offset += b.length; }
    const calls = this.calls;
    this.calls = []; this.blobs = []; this.blobLength = 0; this.gl = null; this.on = false; this.armed = false;
    return { calls, blob };
  }

  private bytes(data: Uint8Array): number {
    // 4-byte alignment keeps typed views cheap on the Rust side.
    const pad = (4 - (this.blobLength % 4)) % 4;
    if (pad) { this.blobs.push(new Uint8Array(pad)); this.blobLength += pad; }
    const offset = this.blobLength;
    this.blobs.push(data.slice()); this.blobLength += data.length;
    return offset;
  }

  private id(object: object): number {
    let id = this.ids.get(object);
    if (!id) { id = this.nextId++; this.ids.set(object, id); }
    return id;
  }

  private typed(view: ArrayBufferView, offsetElements = 0, lengthElements?: number): Arg {
    let tag = 'u8';
    for (const [Ctor, name] of TYPED) if (view instanceof Ctor) { tag = name; break; }
    const element = (view as any).BYTES_PER_ELEMENT || 1;
    const start = view.byteOffset + offsetElements * element;
    const length = lengthElements !== undefined && lengthElements > 0 ? lengthElements * element : view.byteLength - offsetElements * element;
    const data = new Uint8Array(view.buffer, start, length);
    return { b: this.bytes(data), n: length, t: tag };
  }

  private snapshot(source: any): Arg {
    const w = source.naturalWidth || source.videoWidth || source.displayWidth || source.width;
    const h = source.naturalHeight || source.videoHeight || source.displayHeight || source.height;
    if (!this.snap) {
      const canvas = new OffscreenCanvas(1, 1);
      this.snap = canvas.getContext('webgl2', { premultipliedAlpha: false })!;
    }
    const gl = this.snap;
    const texture = gl.createTexture()!, fb = gl.createFramebuffer()!;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, this.unpack.flipY);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, this.unpack.premultiply);
    gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, this.unpack.colorspace);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, source);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    const pixels = new Uint8Array(w * h * 4);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    gl.deleteFramebuffer(fb); gl.deleteTexture(texture);
    return { img: this.bytes(pixels), w, h };
  }

  private encode(value: any): Arg {
    if (value === null || value === undefined) return null;
    // JSON has no Infinity or NaN (three.js passes attenuationDistance = Infinity).
    if (typeof value === 'number' && !Number.isFinite(value)) return { f: String(value) } as any;
    if (typeof value === 'number' || typeof value === 'boolean' || typeof value === 'string') return value;
    if (ArrayBuffer.isView(value)) return this.typed(value);
    if (Array.isArray(value)) return value.map(v => this.encode(v));
    if (typeof value === 'object' && ('naturalWidth' in value || 'displayWidth' in value || value instanceof ImageBitmap
      || value instanceof HTMLCanvasElement || value instanceof OffscreenCanvas || value instanceof ImageData)) return this.snapshot(value);
    if (typeof value === 'object') return { o: this.id(value) };
    throw new Error(`Unrecordable argument ${String(value)}`);
  }

  /** Encodes arguments, folding WebGL2 srcOffset/length overloads into exact slices. */
  private args(name: string, raw: any[]): Arg[] {
    const a = [...raw];
    const sliceAt = (index: number, offsetIndex: number, lengthIndex?: number) => {
      if (ArrayBuffer.isView(a[index]) && a.length > offsetIndex) {
        a[index] = this.typed(a[index], a[offsetIndex] || 0, lengthIndex !== undefined ? a[lengthIndex] : undefined);
        a.length = offsetIndex;
      }
    };
    if (name === 'bufferData') sliceAt(1, 3, 4);
    else if (name === 'bufferSubData') sliceAt(2, 3, 4);
    else if (/^uniform(Matrix)?[1-4](x[2-4])?[fiu]v$/.test(name)) {
      const dataIndex = name.startsWith('uniformMatrix') ? 2 : 1;
      sliceAt(dataIndex, dataIndex + 1, dataIndex + 2);
    } else if (name === 'texImage2D' && a.length === 10) sliceAt(8, 9);
    else if (name === 'texSubImage2D' && a.length === 10) sliceAt(8, 9);
    else if (name === 'texImage3D' && a.length === 11) sliceAt(9, 10);
    else if (name === 'texSubImage3D' && a.length === 12) sliceAt(10, 11);
    return a.map(v => (v && typeof v === 'object' && 'b' in v && 'n' in v ? v : this.encode(v)));
  }

  record(gl: WebGL2RenderingContext, name: string, raw: any[], result: any) {
    if (name === 'pixelStorei') {
      if (raw[0] === gl.UNPACK_FLIP_Y_WEBGL) this.unpack.flipY = !!raw[1];
      if (raw[0] === gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL) this.unpack.premultiply = !!raw[1];
      if (raw[0] === gl.UNPACK_COLORSPACE_CONVERSION_WEBGL) this.unpack.colorspace = raw[1];
    }
    if (/^(draw|clear$)/.test(name)) {
      const size = `${gl.drawingBufferWidth}x${gl.drawingBufferHeight}`;
      if (size !== this.lastSize) { this.calls.push(['__canvas', gl.drawingBufferWidth, gl.drawingBufferHeight]); this.lastSize = size; }
    }
    const entry: any[] = [name, ...this.args(name, raw)];
    if (RETURNS_OBJECT.test(name)) entry.push({ ret: result ? this.id(result) : null });
    else if (name === 'getUniformLocation') entry.push({ ret: result ? this.id(result) : null });
    else if (name === 'getAttribLocation' || name === 'getUniformBlockIndex') entry.push({ ret: result });
    else if (name === 'getActiveAttrib' || name === 'getActiveUniform') entry.push({ ret: result ? { name: result.name, type: result.type, size: result.size } : null });
    this.calls.push(entry);
  }
}

export const recorder = new GlRecorder();

/** Debug probe (render.ts --fs-tail/--fs-match): appends statements to the end of every
 *  fragment `main` whose source contains the match string. The trace records the probed
 *  source, so a replay of the same trace runs the same probe. */
export const shaderDebug: { tail: string; match: string } = { tail: '', match: 'SHADER_TYPE MeshStandardMaterial' };
function debugShader(source: string): string {
  if (!shaderDebug.tail || !source.includes('pc_fragColor') || !source.includes(shaderDebug.match)) return source;
  const end = source.lastIndexOf('}');
  return `${source.slice(0, end)}${shaderDebug.tail}\n}\n`;
}

/** Installs the hooks once; recording only affects the context created while armed. */
export function installRecorder() {
  const proto = WebGL2RenderingContext.prototype as any;
  for (const name of Object.getOwnPropertyNames(proto)) {
    const descriptor = Object.getOwnPropertyDescriptor(proto, name);
    if (!descriptor || typeof descriptor.value !== 'function' || name === 'constructor' || SKIP.has(name)) continue;
    const original = descriptor.value;
    proto[name] = function (this: WebGL2RenderingContext, ...args: any[]) {
      if (name === 'shaderSource') args[1] = debugShader(args[1]);
      const result = original.apply(this, args);
      if (recorder.on && this === recorder.gl) recorder.record(this, name, args, result);
      return result;
    };
  }
  const getContext = HTMLCanvasElement.prototype.getContext as any;
  (HTMLCanvasElement.prototype as any).getContext = function (this: HTMLCanvasElement, type: string, attributes?: any) {
    const context = getContext.call(this, type, attributes);
    if (recorder.armed && !recorder.gl && type === 'webgl2' && context) {
      recorder.gl = context;
      recorder.on = true;
      const attrs = context.getContextAttributes();
      recorder.calls.push(['__context', attrs, context.drawingBufferWidth, context.drawingBufferHeight]);
    }
    return context;
  };
}
