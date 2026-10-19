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
import { DocumentSession, MAX_FALLBACK_BAND_BYTES, type FallbackRunner, type StreamBatch } from './session';
import { toHalfArray } from './half-float';
import { ShaderFallbackError, applyCalls, toHalf, type ShaderFallbackRequest } from './shader-fallback';

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
    expect(logs.some((l) => l.includes('fallback runner could not draw layer L1'))).toBe(true);
  });

  it('reports an unavailable runner rather than leaving the frame waiting', async () => {
    const { port } = await session(null);
    port.deliver(envelope({ kind: 'event', body: { type: 'shaderFallback', fallback: 'requests', requests: [request(4, 4)] } } as never, 5));
    await flush();
    expect(port.fallback()[0]).toMatchObject({ fallback: 'failed', reason: 'unavailable' });
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
});

// All finite binary16 values, including signed zero and subnormals.
it('bulk half conversion preserves every finite binary16 value and view offset', () => {
  const values = new Float32Array(65538);
  for (let bits = 0; bits < 65536; bits++) {
    const sign = bits & 0x8000 ? -1 : 1, exp = bits >>> 10 & 31, frac = bits & 1023;
    values[bits + 1] = exp === 31 ? (frac ? NaN : sign * Infinity)
      : sign * (exp ? (1 + frac / 1024) * 2 ** (exp - 15) : frac * 2 ** -24);
  }
  const converted = toHalfArray(values.subarray(1, 65537));
  for (let bits = 0; bits < 65536; bits++) {
    if ((bits & 0x7c00) === 0x7c00 && (bits & 1023)) expect(converted[bits]! & 0x7fff).toBeGreaterThan(0x7c00);
    else expect(converted[bits]).toBe(bits);
  }
});
