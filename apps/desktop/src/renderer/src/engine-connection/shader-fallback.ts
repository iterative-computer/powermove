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
 * uploaded like the `raster` override.
 *
 * Effects and transitions sample textures: the request names how many
 * (each the target's size, binary16 RGBA in GL row order, sent after the
 * request), which unit samples which (`units`; a `chain` unit samples the
 * previous pass's output from the second pass on), and the calls of every
 * pass after the first (`morePasses`, each into a new target, as the
 * compositor's ping-pong `runEffects`). Textures are RGBA16F, sampled linear
 * with clamp-to-edge like the compositor's `grab` buffers. Units no request
 * names are left empty, so a sampler reads (0, 0, 0, 1), as nothing is bound
 * to one in the compositor either. A parameter call carries `setParam`'s
 * positional alias: applied to the alias when the named uniform is not
 * active.
 *
 * Compiles are asynchronous (`KHR_parallel_shader_compile`) and bounded by a
 * deadline. Every pass's draw is fenced and bounded by the draw deadline
 * (the runner's watchdog): a draw that has not finished by then loses the
 * context on purpose, fails with `watchdog`, and its program is never run
 * again, so a runaway program never blocks the renderer in `readPixels`. A
 * lost context is recreated, and a program whose draw lost it is not run
 * again (it would lose it again).
 */

export type FallbackCallFn = '1f' | '2f' | '3f' | '4f' | '1i' | 'm3';
export interface FallbackCall { fn: FallbackCallFn; name: string; v: number[]; alias?: string }
/** Unit `unit` samples texture `texture` (or, with `chain`, the previous pass's output from the second pass on). */
export interface FallbackUnit { unit: number; texture: number; chain: boolean }

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
  /** Textures sampled (each `width × height`), passed to `render`. */
  textures?: number;
  units?: FallbackUnit[];
  /** Calls of the passes after the first. */
  morePasses?: FallbackCall[][];
}

/** The runner's answer: binary16 RGBA, rows in GL order (row 0 at the bottom). */
export interface ShaderFallbackPixels { key: string; width: number; height: number; rgba16f: Uint16Array }

export class ShaderFallbackError extends Error {
  constructor(readonly key: string, readonly reason: 'compile' | 'deadline' | 'contextLost' | 'unavailable' | 'size' | 'watchdog', message: string) {
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
  /** How long one pass's draw may take before it is stopped (ms): the
   *  runner's watchdog. Chromium's own GPU watchdog loses a context after
   *  about this long, so a draw today's compositor finishes, this finishes. */
  drawDeadlineMs?: number;
  /** The clock and the wait between fence polls (tests). */
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
}

/** Default draw deadline (ms). */
export const DRAW_DEADLINE_MS = 10_000;

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

/** Replays the compositor's uniform calls. A call of the wrong type raises a GL error and changes nothing, as in the compositor. A parameter goes to its positional alias when the named uniform is not active (`setParam`). */
export function applyCalls(gl: GL, locate: (name: string) => WebGLUniformLocation | null, calls: FallbackCall[]): void {
  for (const call of calls) {
    let location = locate(call.name);
    if (location === null && call.alias) location = locate(call.alias);
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

  /** Renders requests one at a time (one context). `textures` are the
   *  request's (binary16 RGBA, GL row order, `width × height × 4` each). */
  render(request: ShaderFallbackRequest, textures: Uint16Array[] = []): Promise<ShaderFallbackPixels> {
    const run = this.queue.then(() => this.run(request, textures));
    this.queue = run.catch(() => undefined);
    return run;
  }

  private now(): number {
    return this.options.now?.() ?? performance.now();
  }

  private sleep(ms: number): Promise<void> {
    return this.options.sleep?.(ms) ?? new Promise((resolve) => setTimeout(resolve, ms));
  }

  /** Waits for a pass's draw to finish, within the draw deadline (the
   *  runner's watchdog). */
  private async finished(gl: GL, id: string, request: ShaderFallbackRequest): Promise<void> {
    const sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
    if (!sync) return;
    gl.flush();
    const deadline = this.now() + (this.options.drawDeadlineMs ?? DRAW_DEADLINE_MS);
    try {
      for (;;) {
        const status = gl.clientWaitSync(sync, 0, 0);
        if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) return;
        if (status === gl.WAIT_FAILED || this.lost || gl.isContextLost()) {
          throw this.fail(id, request, 'compile', 'the program lost the WebGL context when drawn');
        }
        if (this.now() > deadline) {
          const ms = this.options.drawDeadlineMs ?? DRAW_DEADLINE_MS;
          // Abandon the draw: the context goes (and is made again for the
          // next request), the program is never run again.
          const error = this.fail(id, request, 'watchdog', `its draw did not finish within ${Math.round(ms / 1000)} s`);
          gl.getExtension('WEBGL_lose_context')?.loseContext();
          this.lost = true;
          throw error;
        }
        await this.sleep(POLL_MS);
      }
    } finally {
      if (!gl.isContextLost()) gl.deleteSync(sync);
    }
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

  private async program(gl: GL, request: ShaderFallbackRequest): Promise<Program> {
    const id = `${request.vertex}\u0000${request.fragment}`;
    const cached = this.programs.get(id);
    if (cached) {
      this.programs.delete(id);
      this.programs.set(id, cached);
      return cached;
    }
    const failure = this.failed.get(id);
    if (failure) throw new ShaderFallbackError(request.key, failure.reason, failure.message);
    const vs = gl.createShader(gl.VERTEX_SHADER)!, fs = gl.createShader(gl.FRAGMENT_SHADER)!, program = gl.createProgram()!;
    gl.shaderSource(vs, request.vertex); gl.compileShader(vs);
    gl.shaderSource(fs, request.fragment); gl.compileShader(fs);
    gl.attachShader(program, vs); gl.attachShader(program, fs); gl.linkProgram(program);
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

  private async run(request: ShaderFallbackRequest, textures: Uint16Array[]): Promise<ShaderFallbackPixels> {
    const max = this.options.maxSize ?? 4096;
    const { width, height } = request;
    if (!(Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1 && width <= max && height <= max)) {
      throw new ShaderFallbackError(request.key, 'size', `target ${width}×${height} is outside 1..${max}`);
    }
    const wanted = request.textures ?? 0;
    if (textures.length !== wanted || textures.some((t) => t.length !== width * height * 4)) {
      throw new ShaderFallbackError(request.key, 'size', `the request samples ${wanted} textures of ${width}×${height}, ${textures.length} were given`);
    }
    const id = `${request.vertex}\u0000${request.fragment}`;
    const gl = this.context();
    const p = await this.program(gl, request);
    const created: WebGLTexture[] = [];
    const framebuffers: WebGLFramebuffer[] = [];
    const texture = (data: Uint16Array | null): WebGLTexture => {
      const t = gl.createTexture()!;
      created.push(t);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, t);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, width, height, 0, gl.RGBA, gl.HALF_FLOAT, data);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.bindTexture(gl.TEXTURE_2D, null);
      return t;
    };
    try {
      const inputs = textures.map((t) => texture(t));
      const passes = [request.calls, ...(request.morePasses ?? [])];
      let previous: WebGLTexture | null = null;
      for (let pass = 0; pass < passes.length; pass++) {
        const target = texture(null);
        const framebuffer = gl.createFramebuffer()!;
        framebuffers.push(framebuffer);
        gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
        gl.viewport(0, 0, width, height);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.useProgram(p.program);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
        if (p.position >= 0) { gl.enableVertexAttribArray(p.position); gl.vertexAttribPointer(p.position, 2, gl.FLOAT, false, 0, 0); }
        // Units the request does not name stay empty: samplers read (0, 0, 0, 1).
        for (let unit = 0; unit < 16; unit++) { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, null); }
        for (const u of request.units ?? []) {
          const source = u.chain && previous ? previous : inputs[u.texture] ?? null;
          gl.activeTexture(gl.TEXTURE0 + u.unit);
          gl.bindTexture(gl.TEXTURE_2D, source);
        }
        gl.activeTexture(gl.TEXTURE0);
        const locate = (name: string) => {
          if (!p.locations.has(name)) p.locations.set(name, gl.getUniformLocation(p.program, name));
          return p.locations.get(name)!;
        };
        applyCalls(gl, locate, passes[pass]!);
        gl.disable(gl.BLEND);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        // A draw WebGL rejects renders nothing (the target stays transparent), as in the compositor.
        while (gl.getError() !== gl.NO_ERROR) { /* drained */ }
        await this.finished(gl, id, request);
        previous = target;
      }
      const pixels = new Float32Array(width * height * 4);
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.FLOAT, pixels);
      if (this.lost || gl.isContextLost()) {
        throw this.fail(id, request, 'compile', 'the program lost the WebGL context when drawn');
      }
      const rgba16f = new Uint16Array(pixels.length);
      for (let i = 0; i < pixels.length; i++) rgba16f[i] = toHalf(pixels[i]!);
      return { key: request.key, width, height, rgba16f };
    } finally {
      if (!gl.isContextLost()) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        for (let unit = 0; unit < 16; unit++) { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, null); }
        gl.activeTexture(gl.TEXTURE0);
        for (const f of framebuffers) gl.deleteFramebuffer(f);
        for (const t of created) gl.deleteTexture(t);
      }
    }
  }
}
