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
 * Compiles are asynchronous (`KHR_parallel_shader_compile`): a request
 * waits up to a deadline, and a compile past it goes on and is waited for
 * again by the program's next request (the engine asks again), so a large
 * program draws once WebGL has compiled it, as today. Every pass's draw is fenced and bounded by the draw deadline
 * (the runner's watchdog): a draw that has not finished by then loses the
 * context on purpose, fails with `watchdog`, and its program is never run
 * again, so a runaway program never blocks the renderer in `readPixels`.
 *
 * A target edge past the context's `MAX_TEXTURE_SIZE` fails with
 * `tooLarge`: the GPU cannot hold it, today's compositor could not either,
 * and asking again at that size changes nothing (the engine keeps the
 * failure for the program at that size).
 *
 * Only two failures are kept for a program: WebGL refusing it (`compile`)
 * and the draw deadline (`watchdog`). A context lost for any other reason
 * (a GPU-process restart, another context's loss) says nothing about the
 * program, as today's compositor treats it (`compositor.ts` clears its
 * errors and programs on `webglcontextrestored` and draws again): the
 * runner makes a new context and draws the request again once, and if
 * that loses it too fails with `contextLost`, which the engine retries
 * later. A compile past its deadline (`deadline`) is not kept either.
 */

export type FallbackCallFn = '1f' | '2f' | '3f' | '4f' | '1i' | 'm3';
/** One uniform call. `fn` is a `FallbackCallFn`; the protocol's type
 *  (`ShaderFallbackCall`) carries it as a string, and a name the runner
 *  does not know changes nothing (`applyCalls`). */
export interface FallbackCall { fn: FallbackCallFn | (string & {}); name: string; v: number[]; alias?: string }
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
  constructor(readonly key: string, readonly reason: 'compile' | 'deadline' | 'contextLost' | 'unavailable' | 'size' | 'tooLarge' | 'watchdog', message: string) {
    super(message);
  }
}

export interface RunnerOptions {
  /** An offscreen canvas (tests pass their own). */
  createCanvas?: () => OffscreenCanvas | HTMLCanvasElement;
  /** How long a compile may take before the program is failed (ms). */
  compileDeadlineMs?: number;
  /** Largest target edge (tests). By default the context's
   *  `MAX_TEXTURE_SIZE`, the limit today's compositor draws effects and
   *  transitions under (its `grab` buffers are textures of the
   *  composition's size). Shader layers arrive already capped at 4,096 px
   *  by the engine (`shader_layers::plan`, the compositor's own cap). */
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
const MAX_COMPILING = 4;
/** How long one request waits for its program to compile (ms). */
export const COMPILE_DEADLINE_MS = 10_000;
const POLL_MS = 16;

const scratchF32 = new Float32Array(1);
const scratchU32 = new Uint32Array(scratchF32.buffer);

/** f32 to binary16 bits, round to nearest even (values from RGBA16F storage convert exactly). */
export function toHalf(value: number): number {
  scratchF32[0] = value;
  return halfBits(scratchU32[0]!);
}

/** `toHalf` over a whole array, reading the f32 bits in place (no
 *  allocation per value): a full-HD RGBA frame in tens of milliseconds. */
export function toHalfArray(src: Float32Array, dst: Uint16Array = new Uint16Array(src.length)): Uint16Array {
  const bits = new Uint32Array(src.buffer, src.byteOffset, src.length);
  for (let i = 0; i < bits.length; i++) dst[i] = halfBits(bits[i]!);
  return dst;
}

function halfBits(x: number): number {
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
  /** Compiles past a request's deadline, still going in the driver. */
  private compiling = new Map<string, { program: WebGLProgram; vs: WebGLShader; fs: WebGLShader }>();
  private lost = false;
  private queue: Promise<unknown> = Promise.resolve();

  constructor(private readonly options: RunnerOptions = {}) {}

  /** Renders requests one at a time (one context). `textures` are the
   *  request's (binary16 RGBA, GL row order, `width × height × 4` each). */
  render(request: ShaderFallbackRequest, textures: Uint16Array[] = []): Promise<ShaderFallbackPixels> {
    const run = this.queue.then(() => this.attempt(request, textures));
    this.queue = run.catch(() => undefined);
    return run;
  }

  /**
   * Renders one request on the calling thread: the synchronous snapshot
   * path (render-transport.ts), where the thread is blocked on a snapshot
   * that needs these pixels and nothing may wait for a task. The program
   * links as WebGL's `LINK_STATUS` query does (blocking, so no compile
   * deadline), and every pass finishes on this thread before its pixels are read.
   * Chromium's GPU watchdog bounds the blocking driver call, and the
   * elapsed draw deadline is checked before delivering pixels. A context lost
   * under it is made again and the request drawn once more, as `render`.
   */
  renderSync(request: ShaderFallbackRequest, textures: Uint16Array[] = []): ShaderFallbackPixels {
    try {
      return this.runSync(request, textures);
    } catch (error) {
      if (!(error instanceof ShaderFallbackError) || error.reason !== 'contextLost') throw error;
      return this.runSync(request, textures);
    }
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
          this.lost = true;
          throw this.fail(id, request, 'contextLost', 'the WebGL context was lost while the program was drawn');
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

  /** A WebGL fence cannot signal until control returns to the browser's
   *  event loop (WebGL 2, clientWaitSync). A synchronous API cannot yield,
   *  so finish the pass as today's synchronous readPixels does. Chromium's
   *  GPU watchdog bounds the blocking driver call; the elapsed deadline is
   *  checked on return, before any pixels are delivered. */
  private finishedSync(gl: GL, id: string, request: ShaderFallbackRequest): void {
    const started = this.now();
    gl.finish();
    const ms = this.options.drawDeadlineMs ?? DRAW_DEADLINE_MS;
    if (this.now() - started > ms) {
      const error = this.fail(id, request, 'watchdog', `its draw did not finish within ${Math.round(ms / 1000)} s`);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
      this.lost = true;
      throw error;
    }
    if (this.lost || gl.isContextLost()) {
      this.lost = true;
      throw this.fail(id, request, 'contextLost', 'the WebGL context was lost while the program was drawn');
    }
  }

  dispose(): void {
    const gl = this.gl;
    if (gl && !gl.isContextLost()) {
      for (const p of this.programs.values()) gl.deleteProgram(p.program);
      for (const c of this.compiling.values()) { gl.deleteProgram(c.program); gl.deleteShader(c.vs); gl.deleteShader(c.fs); }
      if (this.quad) gl.deleteBuffer(this.quad);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    }
    this.programs.clear();
    this.compiling.clear();
    this.gl = null;
    this.canvas = null;
  }

  private context(): GL {
    if (this.gl && !this.lost && !this.gl.isContextLost()) return this.gl;
    this.programs.clear();
    this.compiling.clear();
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

  /** The program's linked entry (most recently used last), or its kept
   *  failure; null when it has not been linked. */
  private cached(id: string, request: ShaderFallbackRequest): Program | null {
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

  /** The program's compile: one already going on in the driver (a request
   *  past its deadline, or another request waiting for it), or a new one. */
  private start(gl: GL, id: string, request: ShaderFallbackRequest): { program: WebGLProgram; vs: WebGLShader; fs: WebGLShader } {
    const going = this.compiling.get(id);
    if (going) return going;
    const vs = gl.createShader(gl.VERTEX_SHADER)!, fs = gl.createShader(gl.FRAGMENT_SHADER)!, program = gl.createProgram()!;
    gl.shaderSource(vs, request.vertex); gl.compileShader(vs);
    gl.shaderSource(fs, request.fragment); gl.compileShader(fs);
    gl.attachShader(program, vs); gl.attachShader(program, fs); gl.linkProgram(program);
    const started = { program, vs, fs };
    this.compiling.set(id, started);
    return started;
  }

  private async program(gl: GL, request: ShaderFallbackRequest): Promise<Program> {
    const id = `${request.vertex}\u0000${request.fragment}`;
    const cached = this.cached(id, request);
    if (cached) return cached;
    // A compile that passed an earlier request's deadline goes on in the
    // driver: this request waits for the same compile, not a new one, so a
    // program WebGL takes long to compile (today's compositor waits for it)
    // draws once it is done.
    const started = this.start(gl, id, request);
    if (this.parallel) {
      const deadline = this.now() + (this.options.compileDeadlineMs ?? COMPILE_DEADLINE_MS);
      while (!gl.getProgramParameter(started.program, this.parallel.COMPLETION_STATUS_KHR)) {
        if (this.lost || gl.isContextLost()) throw this.fail(id, request, 'contextLost', 'the WebGL context was lost while compiling');
        if (this.now() > deadline) {
          // Kept compiling for the next request (at most a few programs).
          while (this.compiling.size > MAX_COMPILING) {
            const [oldest, c] = [...this.compiling.entries()].find(([key]) => key !== id)!;
            gl.deleteProgram(c.program); gl.deleteShader(c.vs); gl.deleteShader(c.fs);
            this.compiling.delete(oldest);
          }
          throw this.fail(id, request, 'deadline', `the program has not finished compiling after ${Math.round((this.options.compileDeadlineMs ?? COMPILE_DEADLINE_MS) / 1000)} s; it is still compiling and is drawn once it has`);
        }
        await this.sleep(POLL_MS);
        // A synchronous request may have linked (or refused) it meanwhile.
        const done = this.cached(id, request);
        if (done) return done;
      }
    }
    return this.linked(gl, id, request, started);
  }

  /** The program for the synchronous path: `LINK_STATUS` blocks until the
   *  compile (a new one, or one already going on) is done. */
  private programSync(gl: GL, request: ShaderFallbackRequest): Program {
    const id = `${request.vertex}\u0000${request.fragment}`;
    return this.cached(id, request) ?? this.linked(gl, id, request, this.start(gl, id, request));
  }

  private linked(gl: GL, id: string, request: ShaderFallbackRequest, started: { program: WebGLProgram; vs: WebGLShader; fs: WebGLShader }): Program {
    const { program, vs, fs } = started;
    this.compiling.delete(id);
    const linked = gl.getProgramParameter(program, gl.LINK_STATUS);
    const log = (gl.getShaderInfoLog(fs) || '') + (gl.getProgramInfoLog(program) || '');
    // A lost context answers every query with null: not a refusal.
    if (this.lost || gl.isContextLost()) {
      this.lost = true;
      throw this.fail(id, request, 'contextLost', 'the WebGL context was lost while compiling');
    }
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

  /** A failure; kept for the program only when it says something about
   *  it (WebGL refused it, or its draw passed the deadline). */
  private fail(id: string, request: ShaderFallbackRequest, reason: ShaderFallbackError['reason'], message: string): ShaderFallbackError {
    const error = new ShaderFallbackError(request.key, reason, message);
    if (reason === 'compile' || reason === 'watchdog') this.failed.set(id, error);
    return error;
  }

  /** Runs a request; a context lost under it (not by the runner's own
   *  watchdog) is made again and the request drawn once more. */
  private async attempt(request: ShaderFallbackRequest, textures: Uint16Array[]): Promise<ShaderFallbackPixels> {
    try {
      return await this.run(request, textures);
    } catch (error) {
      if (!(error instanceof ShaderFallbackError) || error.reason !== 'contextLost') throw error;
      return this.run(request, textures);
    }
  }

  private async run(request: ShaderFallbackRequest, textures: Uint16Array[]): Promise<ShaderFallbackPixels> {
    const { gl, id } = this.prepare(request, textures);
    const steps = this.draw(gl, await this.program(gl, request), id, request, textures);
    for (let step = steps.next(); ; step = steps.next()) {
      if (step.done) return step.value;
      try {
        await this.finished(gl, id, request);
      } catch (error) {
        steps.return(undefined as never);
        throw error;
      }
    }
  }

  private runSync(request: ShaderFallbackRequest, textures: Uint16Array[]): ShaderFallbackPixels {
    const { gl, id } = this.prepare(request, textures);
    const steps = this.draw(gl, this.programSync(gl, request), id, request, textures);
    for (let step = steps.next(); ; step = steps.next()) {
      if (step.done) return step.value;
      try {
        this.finishedSync(gl, id, request);
      } catch (error) {
        steps.return(undefined as never);
        throw error;
      }
    }
  }

  /** Checks a request against its textures and the context's limit. */
  private prepare(request: ShaderFallbackRequest, textures: Uint16Array[]): { gl: GL; id: string } {
    const { width, height } = request;
    if (!(Number.isInteger(width) && Number.isInteger(height) && width >= 1 && height >= 1)) {
      throw new ShaderFallbackError(request.key, 'size', `target ${width}×${height} is not a size`);
    }
    const wanted = request.textures ?? 0;
    if (textures.length !== wanted || textures.some((t) => t.length !== width * height * 4)) {
      throw new ShaderFallbackError(request.key, 'size', `the request samples ${wanted} textures of ${width}×${height}, ${textures.length} were given`);
    }
    const id = `${request.vertex}\u0000${request.fragment}`;
    const gl = this.context();
    const max = this.options.maxSize ?? Number(gl.getParameter(gl.MAX_TEXTURE_SIZE));
    if (width > max || height > max) {
      throw new ShaderFallbackError(request.key, 'tooLarge', `target ${width}×${height} is larger than this GPU's ${max} px WebGL texture limit`);
    }
    return { gl, id };
  }

  /**
   * Draws a request's passes, yielding after each pass's draw is issued so
   * the caller waits for its fence (`finished` or `finishedSync`), and
   * returns the read-back pixels. Every pass binds all it uses, and the
   * target is bound again before the read: a synchronous request may run
   * on the context between an asynchronous request's passes.
   */
  private *draw(gl: GL, p: Program, id: string, request: ShaderFallbackRequest, textures: Uint16Array[]): Generator<void, ShaderFallbackPixels, void> {
    const { width, height } = request;
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
        yield;
        previous = target;
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffers[framebuffers.length - 1]!);
      // The target's binary16 values as stored: read as half floats when
      // the implementation reads RGBA16F that way (ANGLE does), else as
      // floats, which convert back exactly.
      const values = width * height * 4;
      let rgba16f: Uint16Array;
      if (gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_FORMAT) === gl.RGBA && gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_TYPE) === gl.HALF_FLOAT) {
        rgba16f = new Uint16Array(values);
        gl.readPixels(0, 0, width, height, gl.RGBA, gl.HALF_FLOAT, rgba16f);
      } else {
        const pixels = new Float32Array(values);
        gl.readPixels(0, 0, width, height, gl.RGBA, gl.FLOAT, pixels);
        rgba16f = toHalfArray(pixels);
      }
      if (this.lost || gl.isContextLost()) {
        this.lost = true;
        throw this.fail(id, request, 'contextLost', 'the WebGL context was lost while the program was drawn');
      }
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
