/*
 * The shader fallback runner (plan section 11 and decision 19): a minimal
 * GLSL runner in the renderer for programs WebGL draws and the native
 * engine's pipeline cannot translate (amendment 9: samplers other than a
 * float sampler2D; a naga or Metal compiler failure on a user's machine).
 *
 * One WebGL2 context on an offscreen canvas and the contract only, no
 * compositor: the engine sends the compositor's vertex shader, the wrapped
 * fragment program, the target size, and the uniform calls in issue order
 * (`FallbackRequest`, `engine/crates/pm-render/src/shader_layers/fallback.rs`);
 * the runner draws into an RGBA16F target cleared to transparent with
 * blending off, as the compositor's `grab` buffers, and reads it back in GL
 * row order as binary16. The engine draws the pixels as the layer's content,
 * uploaded like the `raster` override. Texture units are left empty, so a
 * sampler reads (0, 0, 0, 1), as nothing is bound to one in the compositor
 * either.
 *
 * Compiles are asynchronous (`KHR_parallel_shader_compile`) and bounded by a
 * deadline; a lost context is recreated, and a program whose draw lost it is
 * not run again (it would lose it again).
 */

export type FallbackCallFn = '1f' | '2f' | '3f' | '4f' | '1i' | 'm3';
export interface FallbackCall { fn: FallbackCallFn; name: string; v: number[] }

/** `pm_render::shader_layers::FallbackRequest`. */
export interface ShaderFallbackRequest {
  key: string;
  layer: string;
  vertex: string;
  fragment: string;
  width: number;
  height: number;
  calls: FallbackCall[];
  stage: string;
  message: string;
}

/** The runner's answer: binary16 RGBA, rows in GL order (row 0 at the bottom). */
export interface ShaderFallbackPixels { key: string; width: number; height: number; rgba16f: Uint16Array }

export class ShaderFallbackError extends Error {
  constructor(readonly key: string, readonly reason: 'compile' | 'deadline' | 'contextLost' | 'unavailable' | 'size', message: string) {
    super(message);
  }
}

export interface RunnerOptions {
  /** An offscreen canvas (tests pass their own). */
  createCanvas?: () => OffscreenCanvas | HTMLCanvasElement;
  /** How long a compile may take before the program is failed (ms). */
  compileDeadlineMs?: number;
  /** Largest target edge (the compositor's 4,096 px cap on shader layers). */
  maxSize?: number;
}

type GL = WebGL2RenderingContext;
interface Program { program: WebGLProgram; position: number; locations: Map<string, WebGLUniformLocation | null> }

const MAX_PROGRAMS = 32;
const POLL_MS = 16;

/** f32 to binary16 bits, round to nearest even (values from RGBA16F storage convert exactly). */
export function toHalf(value: number): number {
  const f = new Float32Array([value]);
  const x = new Uint32Array(f.buffer)[0]!;
  const sign = (x >>> 16) & 0x8000;
  const exp = (x >>> 23) & 0xff;
  let mant = x & 0x7fffff;
  if (exp === 0xff) return sign | 0x7c00 | (mant ? 0x200 : 0);
  const e = exp - 127 + 15;
  if (e >= 0x1f) return sign | 0x7c00;
  if (e <= 0) {
    if (e < -10) return sign;
    mant |= 0x800000;
    const shift = 14 - e;
    let half = mant >>> shift;
    const rem = mant & ((1 << shift) - 1);
    const mid = 1 << (shift - 1);
    if (rem > mid || (rem === mid && (half & 1))) half++;
    return sign | half;
  }
  let half = (e << 10) | (mant >>> 13);
  const rem = mant & 0x1fff;
  if (rem > 0x1000 || (rem === 0x1000 && (half & 1))) half++;
  return sign | half;
}

/** Replays the compositor's uniform calls. A call of the wrong type raises a GL error and changes nothing, as in the compositor. */
export function applyCalls(gl: GL, locate: (name: string) => WebGLUniformLocation | null, calls: FallbackCall[]): void {
  for (const call of calls) {
    const location = locate(call.name);
    if (location === null) continue;
    const v = call.v;
    switch (call.fn) {
      case '1f': gl.uniform1f(location, v[0]!); break;
      case '2f': gl.uniform2f(location, v[0]!, v[1]!); break;
      case '3f': gl.uniform3f(location, v[0]!, v[1]!, v[2]!); break;
      case '4f': gl.uniform4f(location, v[0]!, v[1]!, v[2]!, v[3]!); break;
      case '1i': gl.uniform1i(location, v[0]!); break;
      case 'm3': gl.uniformMatrix3fv(location, false, new Float32Array(v)); break;
    }
  }
  // Mismatched calls (`uniform1f` on a vec2 or an int) are INVALID_OPERATION.
  while (gl.getError() !== gl.NO_ERROR) { /* drained, as the compositor ignores them */ }
}

export class ShaderFallbackRunner {
  private canvas: OffscreenCanvas | HTMLCanvasElement | null = null;
  private gl: GL | null = null;
  private parallel: { COMPLETION_STATUS_KHR: number } | null = null;
  private quad: WebGLBuffer | null = null;
  private programs = new Map<string, Program>();
  private failed = new Map<string, ShaderFallbackError>();
  private lost = false;
  private queue: Promise<unknown> = Promise.resolve();

  constructor(private readonly options: RunnerOptions = {}) {}

  /** Renders requests one at a time (one context). */
  render(request: ShaderFallbackRequest): Promise<ShaderFallbackPixels> {
    const run = this.queue.then(() => this.run(request));
    this.queue = run.catch(() => undefined);
    return run;
  }

  /**
   * Renders one request on the calling thread (the synchronous snapshot
   * path, render-transport.ts: the thread is blocked on a snapshot that
   * needs these pixels, so nothing may wait for a task). Linking without
   * `KHR_parallel_shader_compile` polling blocks until the program is
   * ready, as WebGL's `LINK_STATUS` query does.
   */
  renderSync(request: ShaderFallbackRequest): ShaderFallbackPixels {
    return this.draw(request, this.programSync(this.context(), request));
  }

  dispose(): void {
    const gl = this.gl;
    if (gl && !gl.isContextLost()) {
      for (const p of this.programs.values()) gl.deleteProgram(p.program);
      if (this.quad) gl.deleteBuffer(this.quad);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    }
    this.programs.clear();
    this.gl = null;
    this.canvas = null;
  }

  private context(): GL {
    if (this.gl && !this.lost && !this.gl.isContextLost()) return this.gl;
    this.programs.clear();
    this.lost = false;
    const canvas = this.options.createCanvas?.() ?? new OffscreenCanvas(4, 4);
    canvas.addEventListener('webglcontextlost', (event) => { event.preventDefault(); this.lost = true; }, false);
    const gl = canvas.getContext('webgl2', { antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: false, depth: false, stencil: false }) as GL | null;
    if (!gl) throw new ShaderFallbackError('', 'unavailable', 'WebGL2 is not available for the shader fallback runner');
    if (!gl.getExtension('EXT_color_buffer_float')) throw new ShaderFallbackError('', 'unavailable', 'EXT_color_buffer_float is not available');
    this.parallel = gl.getExtension('KHR_parallel_shader_compile');
    this.quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.disable(gl.DEPTH_TEST);
    this.canvas = canvas;
    this.gl = gl;
    return gl;
  }

  private cached(request: ShaderFallbackRequest): Program | null {
    const id = `${request.vertex}\u0000${request.fragment}`;
    const cached = this.programs.get(id);
    if (cached) {
      this.programs.delete(id);
      this.programs.set(id, cached);
      return cached;
    }
    const failure = this.failed.get(id);
    if (failure) throw new ShaderFallbackError(request.key, failure.reason, failure.message);
    return null;
  }

  private compile(gl: GL, request: ShaderFallbackRequest): { vs: WebGLShader; fs: WebGLShader; program: WebGLProgram } {
    const vs = gl.createShader(gl.VERTEX_SHADER)!, fs = gl.createShader(gl.FRAGMENT_SHADER)!, program = gl.createProgram()!;
    gl.shaderSource(vs, request.vertex); gl.compileShader(vs);
    gl.shaderSource(fs, request.fragment); gl.compileShader(fs);
    gl.attachShader(program, vs); gl.attachShader(program, fs); gl.linkProgram(program);
    return { vs, fs, program };
  }

  private programSync(gl: GL, request: ShaderFallbackRequest): Program {
    const cached = this.cached(request);
    if (cached) return cached;
    const { vs, fs, program } = this.compile(gl, request);
    return this.linked(gl, request, vs, fs, program);
  }

  private async program(gl: GL, request: ShaderFallbackRequest): Promise<Program> {
    const id = `${request.vertex}\u0000${request.fragment}`;
    const cached = this.cached(request);
    if (cached) return cached;
    const { vs, fs, program } = this.compile(gl, request);
    if (this.parallel) {
      const deadline = performance.now() + (this.options.compileDeadlineMs ?? 10_000);
      while (!gl.getProgramParameter(program, this.parallel.COMPLETION_STATUS_KHR)) {
        if (this.lost || gl.isContextLost()) throw this.fail(id, request, 'contextLost', 'the WebGL context was lost while compiling');
        if (performance.now() > deadline) {
          gl.deleteProgram(program); gl.deleteShader(vs); gl.deleteShader(fs);
          throw this.fail(id, request, 'deadline', 'the program did not compile within the deadline');
        }
        await new Promise((resolve) => setTimeout(resolve, POLL_MS));
      }
    }
    return this.linked(gl, request, vs, fs, program);
  }

  private linked(gl: GL, request: ShaderFallbackRequest, vs: WebGLShader, fs: WebGLShader, program: WebGLProgram): Program {
    const id = `${request.vertex}\u0000${request.fragment}`;
    const linked = gl.getProgramParameter(program, gl.LINK_STATUS);
    const log = (gl.getShaderInfoLog(fs) || '') + (gl.getProgramInfoLog(program) || '');
    gl.deleteShader(vs); gl.deleteShader(fs);
    if (!linked) {
      gl.deleteProgram(program);
      throw this.fail(id, request, 'compile', log.trim() || 'the program does not link');
    }
    const entry: Program = { program, position: gl.getAttribLocation(program, 'a_pos'), locations: new Map() };
    this.programs.set(id, entry);
    while (this.programs.size > MAX_PROGRAMS) {
      const [oldest, p] = this.programs.entries().next().value!;
      gl.deleteProgram(p.program);
      this.programs.delete(oldest);
    }
    return entry;
  }

  private fail(id: string, request: ShaderFallbackRequest, reason: ShaderFallbackError['reason'], message: string): ShaderFallbackError {
    const error = new ShaderFallbackError(request.key, reason, message);
    if (reason !== 'contextLost') this.failed.set(id, error);
    return error;
  }

  private checkSize(request: ShaderFallbackRequest): void {
    const max = this.options.maxSize ?? 4096;
    const { width, height } = request;
    if (!(Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1 && width <= max && height <= max)) {
      throw new ShaderFallbackError(request.key, 'size', `target ${width}×${height} is outside 1..${max}`);
    }
  }

  private async run(request: ShaderFallbackRequest): Promise<ShaderFallbackPixels> {
    this.checkSize(request);
    const gl = this.context();
    return this.draw(request, await this.program(gl, request));
  }

  private draw(request: ShaderFallbackRequest, p: Program): ShaderFallbackPixels {
    this.checkSize(request);
    const gl = this.gl!;
    const { width, height } = request;
    const texture = gl.createTexture()!;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, width, height, 0, gl.RGBA, gl.HALF_FLOAT, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindTexture(gl.TEXTURE_2D, null);
    const framebuffer = gl.createFramebuffer()!;
    try {
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      gl.viewport(0, 0, width, height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.useProgram(p.program);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
      if (p.position >= 0) { gl.enableVertexAttribArray(p.position); gl.vertexAttribPointer(p.position, 2, gl.FLOAT, false, 0, 0); }
      // Nothing is bound to any unit: samplers read (0, 0, 0, 1).
      for (let unit = 0; unit < 16; unit++) { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, null); }
      gl.activeTexture(gl.TEXTURE0);
      const locate = (name: string) => {
        if (!p.locations.has(name)) p.locations.set(name, gl.getUniformLocation(p.program, name));
        return p.locations.get(name)!;
      };
      applyCalls(gl, locate, request.calls);
      gl.disable(gl.BLEND);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      // A draw WebGL rejects renders nothing (the target stays transparent), as in the compositor.
      while (gl.getError() !== gl.NO_ERROR) { /* drained */ }
      const pixels = new Float32Array(width * height * 4);
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.FLOAT, pixels);
      if (this.lost || gl.isContextLost()) {
        throw this.fail(`${request.vertex}\u0000${request.fragment}`, request, 'compile', 'the program lost the WebGL context when drawn');
      }
      const rgba16f = new Uint16Array(pixels.length);
      for (let i = 0; i < pixels.length; i++) rgba16f[i] = toHalf(pixels[i]!);
      return { key: request.key, width, height, rgba16f };
    } finally {
      if (!gl.isContextLost()) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.deleteFramebuffer(framebuffer);
        gl.deleteTexture(texture);
      }
    }
  }
}
