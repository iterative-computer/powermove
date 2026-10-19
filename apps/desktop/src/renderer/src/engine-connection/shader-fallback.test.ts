/*
 * The decision 19 shader fallback runner: the session answers the engine's
 * requests with the runner's pixels in row bands that fit a message (or
 * tells it why not), and the runner replays the compositor's calls with
 * GL's rules. WebGL itself is exercised against the compositor's output by
 * the shader gate (`engine/crates/pm-render/tools/shaders/fallback-runner.ts`
 * and `tests/shader_gate.rs`): vitest has no WebGL.
 */
import { describe, expect, it } from 'vitest';

import { PROTOCOL_BUILD, type Envelope } from '../../../shared/engine-protocol';
import { decodeEnvelope, encodeEnvelope, envelope, localRanges } from '../../../main/engine/protocol';
import { EngineConnection, type PortLike } from './connection';
import { DocumentSession, MAX_FALLBACK_BAND_BYTES, MAX_FALLBACK_WAITING, type FallbackRunner, type StreamBatch } from './session';
import { ShaderFallbackError, ShaderFallbackRunner, applyCalls, toHalf, toHalfArray, type ShaderFallbackRequest } from './shader-fallback';

class FakePort implements PortLike {
  sent: Envelope[] = [];
  private listeners: { message: ((e: { data: unknown }) => void)[]; close: (() => void)[] } = { message: [], close: [] };
  postMessage(message: unknown): void {
    const decoded = decodeEnvelope(message as Uint8Array);
    this.sent.push(decoded);
    const m = decoded.message;
    if (m.kind === 'hello') {
      queueMicrotask(() => this.deliver(envelope({ kind: 'welcome', role: 'interactiveEngine', wire: 1, build: PROTOCOL_BUILD, contracts: localRanges().map((r) => ({ contract: r.contract, version: 0 })), maxMessageBytes: 16 * 1024 * 1024 } as never)));
    } else if (m.kind === 'request') {
      const body = m.body as { type: string; document?: string; revision?: number };
      if (body.type === 'openSession') queueMicrotask(() => this.deliver(envelope({ kind: 'response', id: m.id, body: { type: 'sessionOpened', session: 5 } } as never)));
      else if (body.type === 'resync') queueMicrotask(() => this.deliver(envelope({ kind: 'response', id: m.id, body: { type: 'patchesApplied', document: body.document, revision: body.revision } } as never, 5, body.revision)));
    }
  }
  addEventListener(type: 'message' | 'close', listener: never): void {
    (this.listeners[type] as unknown[]).push(listener);
  }
  start(): void {}
  close(): void {}
  deliver(message: Envelope): void {
    for (const l of this.listeners.message) l({ data: encodeEnvelope(message) });
  }
  fallback(): Array<Record<string, unknown>> {
    return this.sent
      .filter((e) => e.message.kind === 'event' && (e.message.body as { type: string }).type === 'shaderFallback')
      .map((e) => (e.message as { body: Record<string, unknown> }).body);
  }
}

function stream() {
  return {
    ready: true,
    whenReady: Promise.resolve(true),
    subscribe: (_fn: (batch: StreamBatch) => void) => () => {},
    snapshot: () => ({ revision: 3, encoding: 'canonicalJsonV1' as const, json: '{"id":"P"}', hash: 'ab'.repeat(32) }),
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function request(width: number, height: number, key = '5e'.repeat(32)): ShaderFallbackRequest {
  return {
    key, layer: 'L1', vertex: '#version 300 es\nvoid main(){}', fragment: '#version 300 es\nvoid main(){}', width, height,
    calls: [{ fn: '2f', name: 'iResolution', v: [width, height] }], stage: 'bind', message: 'sampler `u` (Cube)',
  };
}

async function session(runner: FallbackRunner | null) {
  const port = new FakePort();
  const connection = new EngineConnection(1, port, () => {});
  await connection.open();
  const logs: string[] = [];
  const s = new DocumentSession({
    connection, stream: stream(), documentId: () => 'P', log: (_l, m) => logs.push(m),
    handlers: { message: () => {}, closed: () => {} },
    fallbackRunner: runner ? () => runner : undefined,
  });
  await s.start();
  await flush();
  return { port, logs };
}

describe('the shader fallback runner in a session', () => {
  it('answers a request with its pixels, in bands below the message limit', async () => {
    const asked: ShaderFallbackRequest[] = [];
    const runner: FallbackRunner = {
      async render(r) {
        asked.push(r);
        const rgba16f = new Uint16Array(r.width * r.height * 4);
        for (let i = 0; i < rgba16f.length; i++) rgba16f[i] = i & 0xffff;
        return { key: r.key, width: r.width, height: r.height, rgba16f };
      }
    };
    const { port } = await session(runner);
    // 2,048 px rows of 16 KiB: 512 rows per 8 MiB band.
    const big = request(2048, 1100);
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests: [big] } } as never, 5));
    await flush();
    expect(asked).toHaveLength(1);
    const bands = port.fallback();
    expect(bands.map((b) => [b['fallback'], b['row'], b['rows']])).toEqual([['pixels', 0, 512], ['pixels', 512, 512], ['pixels', 1024, 76]]);
    for (const band of bands) {
      const bytes = band['rgba16f'] as Uint8Array;
      expect(bytes.byteLength).toBe((band['rows'] as number) * 2048 * 8);
      expect(bytes.byteLength).toBeLessThanOrEqual(MAX_FALLBACK_BAND_BYTES);
    }
    // The second band starts at row 512's first value.
    const second = new Uint16Array((bands[1]!['rgba16f'] as Uint8Array).slice().buffer);
    expect(second[0]).toBe((512 * 2048 * 4) & 0xffff);
  });

  it('tells the engine when the runner cannot draw a request', async () => {
    const runner: FallbackRunner = { render: async (r) => { throw new ShaderFallbackError(r.key, 'compile', 'ERROR: 0:3: syntax error'); } };
    const { port, logs } = await session(runner);
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests: [request(4, 4)] } } as never, 5));
    await flush();
    expect(port.fallback()).toEqual([{ type: 'shaderFallback', fallback: 'failed', key: '5e'.repeat(32), reason: 'compile', message: 'ERROR: 0:3: syntax error' }]);
    expect(logs.some((l) => l.includes('fallback runner could not draw a program of layer L1'))).toBe(true);
  });

  it('reports an unavailable runner rather than leaving the frame waiting', async () => {
    const { port } = await session(null);
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests: [request(4, 4)] } } as never, 5));
    await flush();
    expect(port.fallback()[0]).toMatchObject({ fallback: 'failed', reason: 'unavailable' });
  });
});

describe('effect and transition requests in a session', () => {
  const effect = (width: number, height: number): ShaderFallbackRequest => ({
    ...request(width, height, '7a'.repeat(32)), textures: 2,
    units: [{ unit: 0, texture: 0, chain: true }, { unit: 2, texture: 1, chain: false }], morePasses: []
  });
  const band = (texture: number, row: number, rows: number, width: number, height: number, value: number) => {
    const values = new Uint16Array(width * rows * 4).fill(value);
    return envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'input', key: '7a'.repeat(32), texture, width, height, row, rows, rgba16f: new Uint8Array(values.buffer) } } as never, 5);
  };

  it('waits for every row of every texture, then runs the request with them', async () => {
    const asked: Array<{ r: ShaderFallbackRequest; t: Uint16Array[] }> = [];
    const runner: FallbackRunner = {
      async render(r, t = []) {
        asked.push({ r, t });
        return { key: r.key, width: r.width, height: r.height, rgba16f: new Uint16Array(r.width * r.height * 4) };
      }
    };
    const { port } = await session(runner);
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests: [effect(4, 3)] } } as never, 5));
    port.deliver(band(1, 0, 3, 4, 3, 0x3800));
    port.deliver(band(0, 2, 1, 4, 3, 0x3c00));
    await flush();
    expect(asked).toHaveLength(0);
    port.deliver(band(0, 0, 2, 4, 3, 0x3400));
    await flush();
    expect(asked).toHaveLength(1);
    const [input, backdrop] = asked[0]!.t;
    expect(input![0]).toBe(0x3400);
    expect(input![2 * 16]).toBe(0x3c00);
    expect(backdrop!.every((v) => v === 0x3800)).toBe(true);
    expect(port.fallback()[0]).toMatchObject({ fallback: 'pixels', key: '7a'.repeat(32) });
  });

  it('keeps at most as many requests waiting for textures as the engine does, failing the oldest as transient', async () => {
    const { port } = await session({ render: async () => { throw new Error('not reached'); } });
    const keyed = (i: number): ShaderFallbackRequest => ({ ...effect(4, 3), key: i.toString(16).padStart(64, '0') });
    const requests = Array.from({ length: MAX_FALLBACK_WAITING + 2 }, (_, i) => keyed(i));
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests } } as never, 5));
    await flush();
    const failed = port.fallback().filter((m) => m['fallback'] === 'failed');
    expect(failed.map((m) => m['key'])).toEqual([keyed(0).key, keyed(1).key]);
    expect(failed[0]).toMatchObject({ reason: 'unavailable' });
  });

  it('fails a request whose texture band does not fit it', async () => {
    const { port } = await session({ render: async () => { throw new Error('not reached'); } });
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests: [effect(4, 3)] } } as never, 5));
    port.deliver(band(0, 2, 2, 4, 3, 0));
    await flush();
    expect(port.fallback()[0]).toMatchObject({ fallback: 'failed', reason: 'size' });
  });
});

/** A recording WebGL2 stand-in: enough of the API for the runner. */
function fakeGl(active: string[], fence: () => number, maxTextureSize = 16384) {
  const C = {
    VERTEX_SHADER: 1, FRAGMENT_SHADER: 2, LINK_STATUS: 3, ARRAY_BUFFER: 4, STATIC_DRAW: 5, DEPTH_TEST: 6, BLEND: 7,
    TEXTURE_2D: 8, TEXTURE0: 100, RGBA16F: 9, RGBA: 10, HALF_FLOAT: 11, FLOAT: 12, TEXTURE_MIN_FILTER: 13, TEXTURE_MAG_FILTER: 14,
    LINEAR: 15, TEXTURE_WRAP_S: 16, TEXTURE_WRAP_T: 17, CLAMP_TO_EDGE: 18, FRAMEBUFFER: 19, COLOR_ATTACHMENT0: 20,
    COLOR_BUFFER_BIT: 21, TRIANGLE_STRIP: 22, NO_ERROR: 0, UNPACK_ALIGNMENT: 23, SYNC_GPU_COMMANDS_COMPLETE: 24,
    ALREADY_SIGNALED: 25, CONDITION_SATISFIED: 26, TIMEOUT_EXPIRED: 27, WAIT_FAILED: 28, MAX_TEXTURE_SIZE: 29
  };
  let unit = 0;
  let next = 0;
  let lost = false;
  const bound = new Map<number, unknown>();
  const state = {
    uploads: [] as Array<{ id: number; data: Uint16Array | null }>,
    draws: [] as Array<Record<number, unknown>>,
    uniforms: [] as string[],
    lostBy: 0
  };
  const gl: Record<string, unknown> = {
    ...C,
    getExtension: (name: string) => name === 'EXT_color_buffer_float' ? {} : name === 'WEBGL_lose_context' ? { loseContext: () => { lost = true; state.lostBy++; } } : null,
    createBuffer: () => ({}), bindBuffer: () => {}, bufferData: () => {}, disable: () => {},
    createShader: () => ({}), shaderSource: () => {}, compileShader: () => {}, createProgram: () => ({}), attachShader: () => {}, linkProgram: () => {},
    getProgramParameter: () => true, getShaderInfoLog: () => '', getProgramInfoLog: () => '', deleteShader: () => {}, deleteProgram: () => {},
    getAttribLocation: () => 0,
    createTexture: () => ({ id: next++ }), activeTexture: (u: number) => { unit = u - C.TEXTURE0; }, bindTexture: (_t: number, tex: unknown) => { bound.set(unit, tex); },
    pixelStorei: () => {},
    texImage2D: (...a: unknown[]) => { state.uploads.push({ id: (bound.get(unit) as { id: number }).id, data: a[8] as Uint16Array | null }); },
    texParameteri: () => {}, createFramebuffer: () => ({}), bindFramebuffer: () => {}, framebufferTexture2D: () => {}, viewport: () => {},
    clearColor: () => {}, clear: () => {}, useProgram: () => {}, enableVertexAttribArray: () => {}, vertexAttribPointer: () => {},
    getUniformLocation: (_p: unknown, name: string) => (active.includes(name) ? { name } : null),
    uniform1f: (l: { name: string }, x: number) => state.uniforms.push(`${l.name}=${x}`),
    uniform2f: (l: { name: string }) => state.uniforms.push(l.name), uniform3f: (l: { name: string }) => state.uniforms.push(l.name),
    uniform4f: (l: { name: string }) => state.uniforms.push(l.name), uniform1i: (l: { name: string }, x: number) => state.uniforms.push(`${l.name}=${x}`),
    uniformMatrix3fv: (l: { name: string }) => state.uniforms.push(l.name),
    getError: () => 0,
    drawArrays: () => { state.draws.push(Object.fromEntries([0, 1, 2].map((u) => [u, (bound.get(u) as { id: number } | null)?.id ?? null]))); },
    finish: () => {}, fenceSync: () => ({}), flush: () => {}, clientWaitSync: () => fence(), deleteSync: () => {},
    readPixels: () => {}, isContextLost: () => lost, deleteFramebuffer: () => {}, deleteTexture: () => {},
    getParameter: (p: number) => (p === C.MAX_TEXTURE_SIZE ? maxTextureSize : null)
  };
  const canvas = { addEventListener: () => {}, getContext: () => gl } as unknown as OffscreenCanvas;
  /** The context is lost under the runner (a GPU-process restart). */
  const lose = () => { lost = true; };
  return { canvas, state, C, lose };
}

describe('the runner with textures, passes, and its draw deadline', () => {
  const effect = (calls: ShaderFallbackRequest['calls']): ShaderFallbackRequest => ({
    ...request(2, 1, '7a'.repeat(32)), calls, textures: 2,
    units: [{ unit: 0, texture: 0, chain: true }, { unit: 1, texture: 0, chain: false }, { unit: 2, texture: 1, chain: false }],
    morePasses: [[{ fn: '1i', name: 'u_pass', v: [1] }]]
  });

  it('chains u_tex through the passes, keeps u_orig and u_backdrop, and applies a parameter to its alias', async () => {
    const { canvas, state, C } = fakeGl(['u_tex', 'u_pass', 'u_p0'], () => 25);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    const input = new Uint16Array(8).fill(0x3c00);
    const backdrop = new Uint16Array(8).fill(0x3800);
    const out = await runner.render(effect([{ fn: '1i', name: 'u_tex', v: [0] }, { fn: '1f', name: 'u_amount', v: [0.5], alias: 'u_p0' }]), [input, backdrop]);
    expect(out.rgba16f).toHaveLength(8);
    // Textures 0 and 1 hold the input and the backdrop; 2 and 3 are the passes' targets.
    expect(state.uploads.slice(0, 2).map((u) => u.data)).toEqual([input, backdrop]);
    expect(state.draws).toEqual([{ 0: 0, 1: 0, 2: 1 }, { 0: 2, 1: 0, 2: 1 }]);
    expect(state.uniforms).toEqual(['u_tex=0', 'u_p0=0.5', 'u_pass=1']);
    expect(C.TEXTURE0).toBe(100);
  });

  it('refuses textures that do not fit the request', async () => {
    const { canvas } = fakeGl([], () => 25);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    await expect(runner.render(effect([]), [new Uint16Array(8)])).rejects.toMatchObject({ reason: 'size' });
  });

  it("draws effect targets up to the context's MAX_TEXTURE_SIZE, past the shader layers' 4,096 px cap", async () => {
    const { canvas, state } = fakeGl(['u_tex'], () => 25);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    // An 8K composition: today's compositor draws its effects at this size.
    const r: ShaderFallbackRequest = { ...request(7680, 1, '5e'.repeat(32)), textures: 1, units: [{ unit: 0, texture: 0, chain: false }] };
    const out = await runner.render(r, [new Uint16Array(7680 * 4)]);
    expect(out.width).toBe(7680);
    expect(out.rgba16f).toHaveLength(7680 * 4);
    expect(state.draws).toHaveLength(1);
  });

  it('fails a target past MAX_TEXTURE_SIZE with tooLarge, before compiling or drawing', async () => {
    const { canvas, state } = fakeGl(['u_tex'], () => 25, 8192);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    const r: ShaderFallbackRequest = { ...request(8193, 1, '5f'.repeat(32)), textures: 1, units: [{ unit: 0, texture: 0, chain: false }] };
    await expect(runner.render(r, [new Uint16Array(8193 * 4)])).rejects.toMatchObject({ reason: 'tooLarge', message: expect.stringContaining('8192 px') });
    expect(state.draws).toHaveLength(0);
    // At the limit it draws.
    const fits: ShaderFallbackRequest = { ...r, key: '60'.repeat(32), width: 8192 };
    await expect(runner.render(fits, [new Uint16Array(8192 * 4)])).resolves.toMatchObject({ width: 8192 });
  });

  it('stops a draw past its deadline, loses the context on purpose, and never runs the program again', async () => {
    let clock = 0;
    const { canvas, state } = fakeGl(['u_tex'], () => 27);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas, drawDeadlineMs: 3000, now: () => clock, sleep: async (ms) => { clock += ms * 50; } });
    const r = effect([]);
    await expect(runner.render(r, [new Uint16Array(8), new Uint16Array(8)])).rejects.toMatchObject({ reason: 'watchdog', message: expect.stringContaining('3 s') });
    expect(state.lostBy).toBe(1);
    expect(state.draws).toHaveLength(1);
    await expect(runner.render(r, [new Uint16Array(8), new Uint16Array(8)])).rejects.toMatchObject({ reason: 'watchdog' });
    expect(state.draws).toHaveLength(1);
  });
});

describe('the runner after a lost context', () => {
  const effect = (): ShaderFallbackRequest => ({
    ...request(2, 1, '5c'.repeat(32)), calls: [], textures: 1, units: [{ unit: 0, texture: 0, chain: false }]
  });
  const input = () => [new Uint16Array(8)];

  it('makes a new context and draws again when the context is lost under a draw, keeping nothing for the program', async () => {
    const contexts: Array<ReturnType<typeof fakeGl>> = [];
    let loseNext = 1;
    const runner = new ShaderFallbackRunner({
      createCanvas: () => {
        const ctx: ReturnType<typeof fakeGl> = fakeGl([], () => {
          if (loseNext > 0 && contexts.indexOf(ctx) === 0) { loseNext--; ctx.lose(); return 28; }
          return 25;
        });
        contexts.push(ctx);
        return ctx.canvas;
      }
    });
    const out = await runner.render(effect(), input());
    expect(out.rgba16f).toHaveLength(8);
    expect(contexts).toHaveLength(2);
    expect(contexts[0]!.state.lostBy).toBe(0);
    expect(contexts[1]!.state.draws).toHaveLength(1);
    // The program is not blamed: it draws on the same context from now on.
    await runner.render(effect(), input());
    expect(contexts).toHaveLength(2);
    expect(contexts[1]!.state.draws).toHaveLength(2);
  });

  it('fails with contextLost when the new context is lost too, and draws the program once the GPU is back', async () => {
    let healthy = false;
    let made = 0;
    const runner = new ShaderFallbackRunner({
      createCanvas: () => {
        made++;
        const ctx: ReturnType<typeof fakeGl> = fakeGl([], () => {
          if (!healthy) { ctx.lose(); return 28; }
          return 25;
        });
        return ctx.canvas;
      }
    });
    await expect(runner.render(effect(), input())).rejects.toMatchObject({ reason: 'contextLost' });
    expect(made).toBe(2);
    healthy = true;
    const out = await runner.render(effect(), input());
    expect(out.rgba16f).toHaveLength(8);
  });

  it('does not take a link status read from a lost context for a refusal', async () => {
    let first = true;
    const runner = new ShaderFallbackRunner({
      createCanvas: () => {
        const ctx = fakeGl([], () => 25);
        const gl = (ctx.canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
        if (first) {
          first = false;
          gl['getProgramParameter'] = () => { ctx.lose(); return null; };
        }
        return ctx.canvas;
      }
    });
    const out = await runner.render(effect(), input());
    expect(out.rgba16f).toHaveLength(8);
  });

  it('keeps a refusal: a program WebGL does not link is not compiled again', async () => {
    let links = 0;
    const runner = new ShaderFallbackRunner({
      createCanvas: () => {
        const ctx = fakeGl([], () => 25);
        const gl = (ctx.canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
        gl['linkProgram'] = () => { links++; };
        gl['getProgramParameter'] = () => false;
        gl['getProgramInfoLog'] = () => 'ERROR: 0:1: nope';
        return ctx.canvas;
      }
    });
    await expect(runner.render(effect(), input())).rejects.toMatchObject({ reason: 'compile', message: expect.stringContaining('nope') });
    await expect(runner.render(effect(), input())).rejects.toMatchObject({ reason: 'compile' });
    expect(links).toBe(1);
  });
});

describe('the runner helpers', () => {
  it('replays calls with the GL entry point each names, and drains mismatch errors', () => {
    const log: string[] = [];
    let errors = 2;
    const gl = {
      NO_ERROR: 0,
      uniform1f: (_l: unknown, x: number) => log.push(`1f ${x}`),
      uniform2f: (_l: unknown, x: number, y: number) => log.push(`2f ${x} ${y}`),
      uniform3f: (_l: unknown, ...v: number[]) => log.push(`3f ${v.join(' ')}`),
      uniform4f: (_l: unknown, ...v: number[]) => log.push(`4f ${v.join(' ')}`),
      uniform1i: (_l: unknown, x: number) => log.push(`1i ${x}`),
      uniformMatrix3fv: (_l: unknown, _t: boolean, m: Float32Array) => log.push(`m3 ${m.length}`),
      getError: () => (errors-- > 0 ? 1282 : 0),
    } as unknown as WebGL2RenderingContext;
    applyCalls(gl, (name) => (name === 'absent' ? null : ({} as WebGLUniformLocation)), [
      { fn: 'm3', name: 'u_m', v: [1, 0, 0, 0, 1, 0, 0, 0, 1] },
      { fn: '2f', name: 'iResolution', v: [64, 36] },
      { fn: '1i', name: 'iFrame', v: [30] },
      { fn: '1f', name: 'absent', v: [1] },
      { fn: '3f', name: 'uTint', v: [1, 0.5, 0] },
    ]);
    expect(log).toEqual(['m3 9', '2f 64 36', '1i 30', '3f 1 0.5 0']);
    expect(errors).toBe(-1);
  });

  it('converts to binary16 with round to nearest even', () => {
    expect(toHalf(1)).toBe(0x3c00);
    expect(toHalf(0.5)).toBe(0x3800);
    expect(toHalf(-2)).toBe(0xc000);
    expect(toHalf(65504)).toBe(0x7bff);
    expect(toHalf(1e6)).toBe(0x7c00);
    expect(toHalf(Number.NaN) & 0x7c00).toBe(0x7c00);
    expect(toHalf(1 + 2 ** -11)).toBe(0x3c00);
    expect(toHalf(1 + 3 * 2 ** -11)).toBe(0x3c02);
    expect(toHalf(2 ** -24)).toBe(1);
  });

  it('converts whole arrays exactly as value by value', () => {
    const values = new Float32Array(20_000);
    const bits = new Uint32Array(values.buffer);
    for (let i = 0; i < bits.length; i++) bits[i] = Math.imul(i, 2654435761) >>> 0;
    values.set([1, 0.5, -2, 65504, 1e6, Number.NaN, 1 + 2 ** -11, 2 ** -24, -0, Infinity], 0);
    const expected = Uint16Array.from(values, (v) => toHalf(v));
    expect(toHalfArray(values)).toEqual(expected);
  });
});

describe('the runner reads its target', () => {
  it('as half floats when the implementation reads RGBA16F that way, else as floats', async () => {
    for (const halfRead of [true, false]) {
      const { canvas, C } = fakeGl([], () => 25);
      const gl = (canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
      const reads: number[] = [];
      gl['IMPLEMENTATION_COLOR_READ_FORMAT'] = 30;
      gl['IMPLEMENTATION_COLOR_READ_TYPE'] = 31;
      gl['getParameter'] = (p: number) => (p === 30 ? C.RGBA : p === 31 ? (halfRead ? C.HALF_FLOAT : C.FLOAT) : p === C.MAX_TEXTURE_SIZE ? 16384 : null);
      gl['readPixels'] = (_x: number, _y: number, _w: number, _h: number, _f: number, type: number, out: Float32Array | Uint16Array) => {
        reads.push(type);
        if (type === C.HALF_FLOAT) (out as Uint16Array).fill(0x3800);
        else (out as Float32Array).fill(0.5);
      };
      const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
      const out = await runner.render(request(2, 1, '6d'.repeat(32)));
      expect(reads).toEqual([halfRead ? C.HALF_FLOAT : C.FLOAT]);
      expect(Array.from(out.rgba16f)).toEqual(new Array(8).fill(0x3800));
    }
  });
});

describe('a program WebGL takes long to compile', () => {
  it('is not compiled again: the next request waits for the same compile and draws it', async () => {
    let clock = 0;
    const { canvas } = fakeGl([], () => 25);
    const gl = (canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
    let programs = 0;
    let polls = 0;
    const COMPLETION_STATUS_KHR = 40;
    const getExtension = gl['getExtension'] as (name: string) => unknown;
    gl['getExtension'] = (name: string) => (name === 'KHR_parallel_shader_compile' ? { COMPLETION_STATUS_KHR } : getExtension(name));
    gl['createProgram'] = () => { programs++; return {}; };
    // The compile finishes after 30 polls (about 15 s on the fake clock).
    gl['getProgramParameter'] = (_p: unknown, what: number) => (what === COMPLETION_STATUS_KHR ? ++polls > 30 : true);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas, compileDeadlineMs: 10_000, now: () => clock, sleep: async () => { clock += 500; } });
    const first = request(2, 1, '71'.repeat(32));
    await expect(runner.render(first)).rejects.toMatchObject({ reason: 'deadline', message: expect.stringContaining('still compiling') });
    // Asked again (the engine retries a transient failure): the same
    // compile, which is done within this request's deadline.
    await expect(runner.render({ ...first, key: '72'.repeat(32) })).resolves.toMatchObject({ width: 2 });
    expect(programs).toBe(1);
  });
});

describe('the runner on the calling thread (the synchronous snapshot path)', () => {
  const effect = (): ShaderFallbackRequest => ({
    ...request(2, 1, '7b'.repeat(32)), calls: [{ fn: '1i', name: 'u_tex', v: [0] }], textures: 2,
    units: [{ unit: 0, texture: 0, chain: true }, { unit: 2, texture: 1, chain: false }],
    morePasses: [[{ fn: '1i', name: 'u_pass', v: [1] }]]
  });

  it('draws an effect with its textures and passes, as render does', async () => {
    const { canvas, state } = fakeGl(['u_tex', 'u_pass'], () => 25);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    const input = new Uint16Array(8).fill(0x3c00);
    const backdrop = new Uint16Array(8).fill(0x3800);
    const out = runner.renderSync(effect(), [input, backdrop]);
    expect(out).toMatchObject({ key: '7b'.repeat(32), width: 2, height: 1 });
    expect(state.uploads.slice(0, 2).map((u) => u.data)).toEqual([input, backdrop]);
    expect(state.draws).toEqual([{ 0: 0, 1: null, 2: 1 }, { 0: 2, 1: null, 2: 1 }]);
    // The same program is linked once for both paths.
    await expect(runner.render(effect(), [input, backdrop])).resolves.toMatchObject({ width: 2 });
  });

  it('does not poll a same-task WebGL fence, which the browser cannot signal', () => {
    const { canvas } = fakeGl([], () => { throw new Error('same-task fences cannot signal'); });
    const gl = (canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
    let finished = 0;
    gl['finish'] = () => { finished++; };
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    expect(runner.renderSync(request(2, 1, '8d'.repeat(32)))).toMatchObject({ width: 2 });
    expect(finished).toBe(1);
  });

  it('refuses a request without its textures rather than drawing it unsampled', () => {
    const { canvas, state } = fakeGl([], () => 25);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    expect(() => runner.renderSync(effect())).toThrow(expect.objectContaining({ reason: 'size' }));
    expect(state.draws).toHaveLength(0);
  });

  it('keeps the watchdog: a draw past its deadline loses the context and the program is never run again', () => {
    let clock = 0;
    const { canvas, state } = fakeGl([], () => 27);
    const gl = (canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
    gl['finish'] = () => { clock += 3000; };
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas, drawDeadlineMs: 2000, now: () => clock });
    const r = effect();
    const textures = [new Uint16Array(8), new Uint16Array(8)];
    expect(() => runner.renderSync(r, textures)).toThrow(expect.objectContaining({ reason: 'watchdog' }));
    expect(state.lostBy).toBe(1);
    expect(state.draws).toHaveLength(1);
    expect(() => runner.renderSync(r, textures)).toThrow(expect.objectContaining({ reason: 'watchdog' }));
    expect(state.draws).toHaveLength(1);
  });

  it('fails a target past MAX_TEXTURE_SIZE with tooLarge', () => {
    const { canvas, state } = fakeGl([], () => 25, 4096);
    const runner = new ShaderFallbackRunner({ createCanvas: () => canvas });
    const r: ShaderFallbackRequest = { ...request(4097, 1, '7c'.repeat(32)), textures: 1, units: [{ unit: 0, texture: 0, chain: false }] };
    expect(() => runner.renderSync(r, [new Uint16Array(4097 * 4)])).toThrow(expect.objectContaining({ reason: 'tooLarge' }));
    expect(state.draws).toHaveLength(0);
  });

  it('links a program an asynchronous request is still compiling, without a second compile', async () => {
    let clock = 0;
    const { canvas } = fakeGl([], () => 25);
    const gl = (canvas as unknown as { getContext(): Record<string, unknown> }).getContext();
    let programs = 0;
    const COMPLETION_STATUS_KHR = 40;
    const getExtension = gl['getExtension'] as (name: string) => unknown;
    gl['getExtension'] = (name: string) => (name === 'KHR_parallel_shader_compile' ? { COMPLETION_STATUS_KHR } : getExtension(name));
    gl['createProgram'] = () => { programs++; return {}; };
    // Never complete for the poll: only the blocking LINK_STATUS read finishes it.
    gl['getProgramParameter'] = (_p: unknown, what: number) => (what === COMPLETION_STATUS_KHR ? false : true);
    let wake: () => void = () => {};
    const runner = new ShaderFallbackRunner({
      createCanvas: () => canvas, compileDeadlineMs: 10_000, now: () => clock,
      sleep: () => new Promise<void>((resolve) => { wake = () => { clock += 16; resolve(); }; })
    });
    const r = request(2, 1, '7d'.repeat(32));
    const pending = runner.render(r);
    await flush();
    // A snapshot needs the same program meanwhile.
    expect(runner.renderSync({ ...r, key: '7e'.repeat(32) })).toMatchObject({ width: 2 });
    wake();
    await expect(pending).resolves.toMatchObject({ key: '7d'.repeat(32) });
    expect(programs).toBe(1);
  });
});
