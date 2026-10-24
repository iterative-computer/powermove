export interface HalfReadbackSession {
  capture(time: number, options?: Record<string, unknown>): Uint8Array<ArrayBuffer> | null;
  dispose(): void;
}

export interface DeferredAlphaReader {
  queue(texture: WebGLTexture): void;
}

export interface AlphaReadbackTicket {
  readInto(target: Uint8Array, cancelled: () => boolean): Promise<boolean>;
  dispose(): void;
}

export interface AlphaReadbackSession {
  readonly byteLength: number;
  begin(time: number, options?: Record<string, unknown>): AlphaReadbackTicket | null;
  dispose(): void;
}

/** The queued accumulator is gone; a healthy GPU must recapture the same time
 * through the existing synchronous path. This is never user cancellation. */
export class AlphaReadbackRetryError extends Error {
  constructor() { super('The GPU alpha frame must be captured again'); }
}

const gateResults = new WeakMap<object, boolean>();
const floatGateResults = new WeakMap<object, boolean>();
const VERTEX = `#version 300 es
precision highp float;
in vec2 a_pos;
void main(){gl_Position=vec4(a_pos*2.-1.,0.,1.);}`;
const FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D u_tex;
uniform highp uint u_height;
uniform highp uvec4 u_nonce;
layout(location=0) out uvec4 o;
float quantize(float value){
  if(isnan(value))return 0.;
  return floor(clamp(value,0.,1.)*255.+.5);
}
void main(){
  if(uint(gl_FragCoord.y)>=u_height){o=u_nonce;return;}
  vec4 c=texelFetch(u_tex,ivec2(gl_FragCoord.xy),0);
  o=uvec4(quantize(c.r),quantize(c.g),quantize(c.b),quantize(c.a));
}`;

const FLOAT_FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D u_tex;
uniform highp uint u_height;
uniform highp uvec4 u_nonce;
layout(location=0) out uvec4 o;
uint quantize(highp float value){
  highp uint bits=floatBitsToUint(value);
  highp uint exponent=(bits>>23u)&255u;
  highp uint fraction=bits&0x7fffffu;
  if((bits&0x80000000u)!=0u)return 0u;
  if(exponent==255u&&fraction!=0u)return 0u;
  if(exponent>=127u)return 255u;
  if(exponent<=117u)return 0u;
  highp uint product=(fraction|0x800000u)*255u;
  if(exponent==118u)return product>=0x80000000u?1u:0u;
  highp uint shift=150u-exponent;
  highp uint remainder=product&((1u<<shift)-1u);
  return (product>>shift)+(remainder>=(1u<<(shift-1u))?1u:0u);
}
void main(){
  if(uint(gl_FragCoord.y)>=u_height){o=u_nonce;return;}
  highp vec4 c=texelFetch(u_tex,ivec2(gl_FragCoord.xy),0);
  o=uvec4(quantize(c.r),quantize(c.g),quantize(c.b),quantize(c.a));
}`;

/** Integer arithmetic matches JavaScript's double-precision multiply and round.
 * Float32 multiplication can round across a byte midpoint before the rounding
 * step. The significand times 255 fits uint32; exponent 118 avoids a shift of
 * 32, which GLSL leaves undefined. The other shifts stay between 24 and 31. */
function floatProbe(width: number, height: number): Float32Array<ArrayBuffer> {
  const edges: number[] = [], scratch = new Float32Array(1), encoded = new Uint32Array(scratch.buffer);
  for (let byte = 0; byte < 255; byte++) {
    scratch[0] = (byte + .5) / 255;
    for (let neighbour = -16; neighbour <= 16; neighbour++) {
      const bits = encoded[0]! + neighbour;
      edges.push(bits, bits | 0x80000000);
    }
  }
  for (let exponent = 0; exponent < 256; exponent++) {
    for (const fraction of [0, 1, 0x3fffff, 0x400000, 0x7ffffe, 0x7fffff]) {
      const bits = exponent * 0x800000 + fraction;
      edges.push(bits, bits | 0x80000000);
    }
  }
  const count = width * height, samples = new Uint32Array(count);
  let random = 0x6d2b79f5;
  for (let i = 0; i < count; i++) {
    random ^= random << 13; random ^= random >>> 17; random ^= random << 5;
    samples[i] = i < edges.length ? edges[i]! : random;
  }
  const floats = new Float32Array(count * 4), bits = new Uint32Array(floats.buffer);
  for (let pixel = 0; pixel < count; pixel++) {
    bits[pixel * 4] = samples[pixel]!;
    bits[pixel * 4 + 1] = samples[(pixel * 251 + 12345) % count]!;
    bits[pixel * 4 + 2] = samples[count - 1 - pixel]!;
    bits[pixel * 4 + 3] = samples[(pixel * 40503 + 17) % count]!;
  }
  return floats;
}

function saveState(gl: WebGL2RenderingContext) {
  const active = gl.getParameter(gl.ACTIVE_TEXTURE);
  let texture, sampler;
  try { gl.activeTexture(gl.TEXTURE0); texture = gl.getParameter(gl.TEXTURE_BINDING_2D); sampler = gl.getParameter(gl.SAMPLER_BINDING); }
  finally { gl.activeTexture(active); }
  return {
    active, texture, sampler, program: gl.getParameter(gl.CURRENT_PROGRAM), vao: gl.getParameter(gl.VERTEX_ARRAY_BINDING),
    array: gl.getParameter(gl.ARRAY_BUFFER_BINDING), read: gl.getParameter(gl.READ_FRAMEBUFFER_BINDING),
    draw: gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING), renderbuffer: gl.getParameter(gl.RENDERBUFFER_BINDING),
    viewport: gl.getParameter(gl.VIEWPORT), mask: gl.getParameter(gl.COLOR_WRITEMASK),
    enabled: [gl.BLEND, gl.DITHER, gl.DEPTH_TEST, gl.SCISSOR_TEST, gl.CULL_FACE, gl.STENCIL_TEST,
      gl.RASTERIZER_DISCARD, gl.SAMPLE_ALPHA_TO_COVERAGE, gl.SAMPLE_COVERAGE].map(cap => [cap, gl.isEnabled(cap)] as const),
    pack: gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING), unpack: gl.getParameter(gl.PIXEL_UNPACK_BUFFER_BINDING),
    stores: [gl.PACK_ALIGNMENT, gl.PACK_ROW_LENGTH, gl.PACK_SKIP_ROWS, gl.PACK_SKIP_PIXELS,
      gl.UNPACK_ALIGNMENT, gl.UNPACK_ROW_LENGTH, gl.UNPACK_SKIP_ROWS, gl.UNPACK_SKIP_PIXELS,
      gl.UNPACK_FLIP_Y_WEBGL, gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL].map(key => [key, gl.getParameter(key)] as const),
  };
}

function restoreState(gl: WebGL2RenderingContext, state: ReturnType<typeof saveState>) {
  gl.useProgram(state.program); gl.bindVertexArray(state.vao); gl.bindBuffer(gl.ARRAY_BUFFER, state.array);
  gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, state.texture); gl.bindSampler(0, state.sampler); gl.activeTexture(state.active);
  gl.bindFramebuffer(gl.READ_FRAMEBUFFER, state.read); gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, state.draw);
  gl.bindRenderbuffer(gl.RENDERBUFFER, state.renderbuffer);
  gl.viewport(...state.viewport as [number, number, number, number]);
  gl.colorMask(...state.mask as [boolean, boolean, boolean, boolean]);
  for (const [cap, enabled] of state.enabled) { if (enabled) gl.enable(cap); else gl.disable(cap); }
  gl.bindBuffer(gl.PIXEL_PACK_BUFFER, state.pack);
  gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, state.unpack);
  for (const [key, value] of state.stores) gl.pixelStorei(key, value);
}

/** An export owns its shader, integer target and reusable raw bytes. Only the
 * compatibility result survives it, keyed by a GL.init generation
 * token: restoration can reuse the same WebGL context object. */
export function createHalfReadback(
  gl: WebGL2RenderingContext | null,
  width: number,
  height: number,
  generation: object,
  render: (time: number, options: Record<string, unknown>, readHalf: (texture: WebGLTexture) => Uint8Array<ArrayBuffer> | null) => Uint8Array<ArrayBuffer> | null,
  valid: () => boolean,
): HalfReadbackSession | null {
  return createReadback(gl, width, height, generation, render, valid, 16);
}

export function createFloatReadback(
  gl: WebGL2RenderingContext | null,
  width: number,
  height: number,
  generation: object,
  render: (time: number, options: Record<string, unknown>, readFloat: (texture: WebGLTexture) => Uint8Array<ArrayBuffer> | null) => Uint8Array<ArrayBuffer> | null,
  valid: () => boolean,
): HalfReadbackSession | null {
  return createReadback(gl, width, height, generation, render, valid, 32);
}

function createReadback(
  gl: WebGL2RenderingContext | null,
  width: number,
  height: number,
  generation: object,
  render: (time: number, options: Record<string, unknown>, read: (texture: WebGLTexture) => Uint8Array<ArrayBuffer> | null) => Uint8Array<ArrayBuffer> | null,
  valid: () => boolean,
  depth: 16 | 32,
): HalfReadbackSession | null {
  const backend = createReadbackBackend(gl, width, height, generation, valid, depth);
  if (!backend) return null;
  let capturing = false;
  return {
    capture(time, options = {}) {
      if (!backend.available()) throw new Error('The GPU could not capture the export frame');
      if (capturing) throw new Error(depth === 16 ? 'A half-float capture is already in progress' : 'A float capture is already in progress');
      capturing = true;
      try { return render(time, options, backend.readTexture); }
      finally { capturing = false; }
    },
    dispose: backend.dispose,
  };
}

function createReadbackBackend(
  gl: WebGL2RenderingContext | null,
  width: number,
  height: number,
  generation: object,
  valid: () => boolean,
  depth: 16 | 32,
) {
  const length = width * height * 4;
  const gates = depth === 16 ? gateResults : floatGateResults;
  if (!gl || !Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0
      || !Number.isSafeInteger(width * (height + 1) * 4) || !valid() || gl.isContextLost() || gates.get(generation) === false) return null;
  // The extra guard row must fit too. A size limit says nothing about whether
  // this context can quantize smaller captures, so do not cache this rejection.
  const maximum = gl.getParameter(gl.MAX_RENDERBUFFER_SIZE);
  if (width > maximum || height >= maximum) return null;
  const precision = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_FLOAT);
  const integer = depth === 32 ? gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_INT) : null;
  if (!precision || precision.precision < 23 || precision.rangeMin < (depth === 32 ? 126 : 24)
      || precision.rangeMax < (depth === 32 ? 127 : 8)
      || depth === 32 && (!integer || integer.rangeMin < 31 || integer.rangeMax < 30)) {
    gates.set(generation, false); return null;
  }
  let program: WebGLProgram | null = null, vao: WebGLVertexArrayObject | null = null, quad: WebGLBuffer | null = null;
  let framebuffer: WebGLFramebuffer | null = null, color: WebGLRenderbuffer | null = null;
  let pixels: Uint8Array<ArrayBuffer> | null = null, output: Uint8Array<ArrayBuffer> | null = null;
  let targetWidth = 0, targetHeight = 0, nonce = 0, disposed = false, disabled = false;
  const shaders: WebGLShader[] = [];
  const available = () => !disposed && valid() && !gl.isContextLost();
  const failure = () => new Error('The GPU could not capture the export frame');
  const release = () => {
    if (program) gl.deleteProgram(program); program = null;
    for (const shader of shaders) gl.deleteShader(shader); shaders.length = 0;
    if (vao) gl.deleteVertexArray(vao); vao = null;
    if (quad) gl.deleteBuffer(quad); quad = null;
    if (framebuffer) gl.deleteFramebuffer(framebuffer); framebuffer = null;
    if (color) gl.deleteRenderbuffer(color); color = null;
    pixels = output = null; targetWidth = targetHeight = 0;
  };
  const allocate = (w: number, h: number) => {
    if (!program) {
      const compile = (type: number, source: string) => {
        const shader = gl.createShader(type);
        if (!shader) throw failure();
        shaders.push(shader); gl.shaderSource(shader, source); gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw failure();
        return shader;
      };
      const vertex = compile(gl.VERTEX_SHADER, VERTEX), fragment = compile(gl.FRAGMENT_SHADER, depth === 16 ? FRAGMENT : FLOAT_FRAGMENT);
      program = gl.createProgram();
      if (!program) throw failure();
      gl.attachShader(program, vertex); gl.attachShader(program, fragment); gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw failure();
      vao = gl.createVertexArray(); quad = gl.createBuffer(); framebuffer = gl.createFramebuffer(); color = gl.createRenderbuffer();
      if (!vao || !quad || !framebuffer || !color) throw failure();
      gl.bindVertexArray(vao); gl.bindBuffer(gl.ARRAY_BUFFER, quad);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
      const position = gl.getAttribLocation(program, 'a_pos');
      if (position < 0 || gl.getBufferParameter(gl.ARRAY_BUFFER, gl.BUFFER_SIZE) !== 32) throw failure();
      gl.enableVertexAttribArray(position); gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    }
    if (targetWidth !== w || targetHeight !== h) {
      gl.bindRenderbuffer(gl.RENDERBUFFER, color); gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8UI, w, h + 1);
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, color);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE
          || gl.getRenderbufferParameter(gl.RENDERBUFFER, gl.RENDERBUFFER_WIDTH) !== w
          || gl.getRenderbufferParameter(gl.RENDERBUFFER, gl.RENDERBUFFER_HEIGHT) !== h + 1
          || gl.getRenderbufferParameter(gl.RENDERBUFFER, gl.RENDERBUFFER_INTERNAL_FORMAT) !== gl.RGBA8UI
          || gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_FORMAT) !== gl.RGBA_INTEGER
          || gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_TYPE) !== gl.UNSIGNED_BYTE) throw failure();
      targetWidth = w; targetHeight = h;
    }
  };
  const draw = (texture: WebGLTexture, w: number, h: number, bytes: Uint8Array | null,
    queue?: (stamp: readonly number[]) => void): boolean => {
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer); gl.viewport(0, 0, w, h + 1);
    for (const cap of [gl.BLEND, gl.DITHER, gl.DEPTH_TEST, gl.SCISSOR_TEST, gl.CULL_FACE, gl.STENCIL_TEST,
      gl.RASTERIZER_DISCARD, gl.SAMPLE_ALPHA_TO_COVERAGE, gl.SAMPLE_COVERAGE]) gl.disable(cap);
    gl.colorMask(true, true, true, true); gl.useProgram(program); gl.bindVertexArray(vao);
    gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, texture); gl.bindSampler(0, null); gl.uniform1i(gl.getUniformLocation(program!, 'u_tex'), 0);
    nonce = nonce % 255 + 1;
    const stamp = [nonce, 256 - nonce, nonce * 17 % 255 + 1, nonce * 73 % 255 + 1] as const;
    gl.uniform1ui(gl.getUniformLocation(program!, 'u_height'), h);
    gl.uniform4ui(gl.getUniformLocation(program!, 'u_nonce'), ...stamp);
    // A fresh nonzero guard row proves both drawing and reading succeeded,
    // without consuming unrelated errors already in WebGL's error queue.
    bytes?.fill(0, w * h * 4);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null); gl.pixelStorei(gl.PACK_ALIGNMENT, 4);
    for (const key of [gl.PACK_ROW_LENGTH, gl.PACK_SKIP_ROWS, gl.PACK_SKIP_PIXELS]) gl.pixelStorei(key, 0);
    if (queue) { queue(stamp); return true; }
    gl.readPixels(0, 0, w, h + 1, gl.RGBA_INTEGER, gl.UNSIGNED_BYTE, bytes!);
    if (!available()) throw failure();
    for (let i = w * h * 4; i < bytes!.length; i++) if (bytes![i] !== stamp[i % 4]) return false;
    return true;
  };
  const probe = () => {
    const cached = gates.get(generation);
    if (cached !== undefined) return cached;
    let source: WebGLTexture | null = null, sourceFramebuffer: WebGLFramebuffer | null = null, passed = false;
    try {
      // Odd rows exercise addressing with unrelated neighbours and channel
      // permutations. The 16-bit probe covers every half-float bit pattern;
      // the 32-bit probe samples rounding boundaries and representation classes.
      const w = 257, h = 256, count = w * h * 4;
      allocate(w, h);
      source = gl.createTexture(); sourceFramebuffer = gl.createFramebuffer();
      if (!source || !sourceFramebuffer) return false;
      const bits = depth === 16 ? new Uint16Array(count) : floatProbe(w, h);
      if (depth === 16) for (let pixel = 0; pixel < w * h; pixel++) {
        bits[pixel * 4] = pixel & 65535;
        bits[pixel * 4 + 1] = (pixel * 251 + 12345) & 65535;
        bits[pixel * 4 + 2] = (pixel ^ 32768) & 65535;
        bits[pixel * 4 + 3] = (pixel * 40503 + 17) & 65535;
      }
      gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, source);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
      gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, null);
      for (const key of [gl.UNPACK_ROW_LENGTH, gl.UNPACK_SKIP_ROWS, gl.UNPACK_SKIP_PIXELS, gl.UNPACK_FLIP_Y_WEBGL, gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL]) gl.pixelStorei(key, 0);
      gl.texImage2D(gl.TEXTURE_2D, 0, depth === 16 ? gl.RGBA16F : gl.RGBA32F, w, h, 0, gl.RGBA, depth === 16 ? gl.HALF_FLOAT : gl.FLOAT, bits);
      gl.bindFramebuffer(gl.FRAMEBUFFER, sourceFramebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, source, 0);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) return false;
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null); gl.pixelStorei(gl.PACK_ALIGNMENT, 4);
      for (const key of [gl.PACK_ROW_LENGTH, gl.PACK_SKIP_ROWS, gl.PACK_SKIP_PIXELS]) gl.pixelStorei(key, 0);
      const floats = new Float32Array(count), expected = new Uint8Array(count), actual = new Uint8Array(w * (h + 1) * 4);
      floats.fill(NaN);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.FLOAT, floats);
      if (!available() || !draw(source, w, h, actual)) return false;
      for (let i = 0; i < count; i++) {
        expected[i] = Math.round(Math.max(0, Math.min(1, floats[i]!)) * 255);
        if (actual[i] !== expected[i]) return false;
      }
      passed = true; return true;
    } catch { return false; }
    finally {
      if (source) gl.deleteTexture(source);
      if (sourceFramebuffer) gl.deleteFramebuffer(sourceFramebuffer);
      gates.set(generation, passed);
    }
  };
  const initialize = () => {
    if (!probe()) { disabled = true; release(); if (!available()) throw failure(); return false; }
    try { allocate(width, height); }
    catch { disabled = true; release(); if (!available()) throw failure(); return false; }
    return true;
  };
  const readTexture = (texture: WebGLTexture): Uint8Array<ArrayBuffer> | null => {
    if (!available()) throw failure();
    if (disabled) return null;
    const state = saveState(gl);
    try {
      if (!initialize()) return null;
      try {
        if (!output) { output = new Uint8Array(width * (height + 1) * 4); pixels = output.subarray(0, length); }
      } catch {
        disabled = true; release(); if (!available()) throw failure(); return null;
      }
      try {
        if (!draw(texture, width, height, output!)) { disabled = true; release(); return null; }
      } catch {
        disabled = true; release(); if (!available()) throw failure(); return null;
      }
      return pixels;
    } finally { restoreState(gl, state); }
  };
  return {
    available,
    readTexture,
    prepare() {
      if (!available()) throw failure();
      const state = saveState(gl);
      try { return !disabled && initialize(); }
      finally { restoreState(gl, state); }
    },
    queueTexture(texture: WebGLTexture, queue: (stamp: readonly number[]) => void) {
      if (!available() || disabled) throw failure();
      const state = saveState(gl);
      try { draw(texture, width, height, null, queue); }
      finally { restoreState(gl, state); }
    },
    dispose() { if (!disposed) { disposed = true; release(); } },
  };
}

/** The integer target and one PBO are export-owned. CPU storage belongs to the
 * caller and is filled only after its preceding native write acknowledges it. */
export function createAlphaReadback(
  gl: WebGL2RenderingContext | null,
  width: number,
  height: number,
  generation: object,
  depth: 16 | 32,
  render: (time: number, options: Record<string, unknown>, read: DeferredAlphaReader) => void,
  valid: () => boolean,
  matchesDepth: () => boolean,
): AlphaReadbackSession | null {
  const byteLength = width * (height + 1) * 4, visible = width * height * 4;
  if (!gl || !Number.isSafeInteger(byteLength) || byteLength > 32 * 1024 * 1024
      || !gl.fenceSync || !gl.clientWaitSync || !gl.getBufferSubData || !matchesDepth()) return null;
  const backend = createReadbackBackend(gl, width, height, generation, valid, depth);
  if (!backend) return null;
  let buffer: WebGLBuffer | null = null;
  const previous = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
  try {
    if (!backend.prepare()) { backend.dispose(); return null; }
    buffer = gl.createBuffer();
    if (!buffer) { backend.dispose(); return null; }
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
    gl.bufferData(gl.PIXEL_PACK_BUFFER, byteLength, gl.STREAM_READ);
    if (gl.getBufferParameter(gl.PIXEL_PACK_BUFFER, gl.BUFFER_SIZE) !== byteLength || !backend.available()) {
      gl.deleteBuffer(buffer); buffer = null;
    }
  } catch {
    if (buffer) gl.deleteBuffer(buffer);
    buffer = null;
  } finally { gl.bindBuffer(gl.PIXEL_PACK_BUFFER, previous); }
  if (!buffer) { backend.dispose(); return null; }

  let disposed = false, disabled = false, pending: AlphaReadbackTicket | null = null;
  const available = () => !disposed && backend.available();
  const failure = () => new Error('The GPU could not capture the export frame');
  return {
    byteLength,
    begin(time, options = {}) {
      if (!available()) throw failure();
      if (pending) throw new Error('A GPU alpha frame capture is already pending');
      if (disabled || !matchesDepth()) return null;
      let fence: WebGLSync | null = null, stamp: readonly number[] | null = null;
      let reading = false, finished = false;
      let timer: ReturnType<typeof setTimeout> | undefined, wake: (() => void) | undefined;
      const finish = () => {
        if (finished) return;
        finished = true; clearTimeout(timer);
        if (fence) gl.deleteSync(fence);
        fence = null;
        if (pending === ticket) pending = null;
      };
      const retry = () => { disabled = true; throw new AlphaReadbackRetryError(); };
      const ticket: AlphaReadbackTicket = {
        async readInto(target, cancelled) {
          if (finished || disposed) { finish(); return false; }
          if (reading) throw new Error('A GPU alpha frame capture is already being read');
          reading = true;
          try {
            if (cancelled()) return false;
            if (!(target instanceof Uint8Array) || target.length !== byteLength) throw new Error('The GPU capture target has the wrong size or type');
            if (!available()) throw failure();
            const started = performance.now();
            const ready = await new Promise<boolean>((resolve, reject) => {
              const settle = (result: boolean, error?: unknown) => {
                clearTimeout(timer); wake = undefined;
                if (error !== undefined) reject(error); else resolve(result);
              };
              wake = () => settle(false);
              const poll = () => {
                try {
                  if (finished || disposed || cancelled()) { settle(false); return; }
                  if (!available()) throw failure();
                  if (performance.now() - started >= 10000) throw new Error('The GPU frame capture timed out');
                  const status = gl.clientWaitSync(fence!, 0, 0);
                  if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) settle(true);
                  else if (status === gl.WAIT_FAILED) throw failure();
                  else timer = setTimeout(poll, 0);
                } catch (error) { settle(false, error instanceof Error ? error : failure()); }
              };
              // A WebGL fence cannot signal until the browser reaches another
              // event-loop task; awaiting only microtasks cannot make progress.
              timer = setTimeout(poll, 0);
            });
            if (!ready || finished || disposed || cancelled()) return false;
            if (!available() || target.length !== byteLength) throw failure();
            target.fill(0, visible);
            const old = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
            try {
              gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
              gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, target);
            } catch {
              if (!available()) throw failure();
              retry();
            } finally { gl.bindBuffer(gl.PIXEL_PACK_BUFFER, old); }
            if (!available()) throw failure();
            for (let i = visible; i < byteLength; i++) if (target[i] !== stamp![i % 4]) retry();
            return true;
          } finally { wake = undefined; finish(); }
        },
        dispose() { wake?.(); finish(); },
      };
      pending = ticket;
      try {
        render(time, options, { queue(texture) {
          if (finished || !available()) throw failure();
          if (stamp) throw new Error('A GPU alpha frame capture was queued twice');
          backend.queueTexture(texture, nonce => {
            stamp = nonce;
            gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
            gl.readPixels(0, 0, width, height + 1, gl.RGBA_INTEGER, gl.UNSIGNED_BYTE, 0);
            fence = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
            if (!fence) throw failure();
            gl.flush();
          });
        } });
        if (!available()) throw failure();
        if (!stamp || !matchesDepth()) { ticket.dispose(); return null; }
        return ticket;
      } catch (error) { ticket.dispose(); throw error; }
    },
    dispose() {
      if (disposed) return;
      disposed = true; pending?.dispose();
      gl.deleteBuffer(buffer); buffer = null;
      backend.dispose();
    },
  };
}
