/*
 * The job engine's fallback requests in main (contract 14): the textures
 * are read from the session's scratch area, the request runs in the
 * window the job works for (else any window), and the result is written
 * back; failures come back as the codes the job engine maps to the
 * runner's reasons.
 */
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import type { HostShaderFallback, ShaderFallbackRequest } from '../../shared/engine-protocol';
import { ProtocolError } from './protocol';
import { ShaderFallbackRelay, fallbackErrorCode, serveHostFallback, type ShaderFallbackJob } from './shader-fallback-relay';

const request = (textures = 2): ShaderFallbackRequest => ({
  key: '7a'.repeat(32), layer: 'L', vertex: 'v', fragment: 'f', width: 2, height: 1,
  calls: [], stage: 'bind', message: 'sampler', textures, units: [], morePasses: []
});

const flush = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

function relay(windows: number[], owner: number | null) {
  const sent: Array<{ window: number; job: ShaderFallbackJob }> = [];
  const timers: Array<() => void> = [];
  const acks: Array<() => void> = [];
  const r = new ShaderFallbackRelay({
    send: (window, job) => {
      if (window === 404) return false;
      sent.push({ window, job });
      return true;
    },
    windows: () => windows,
    ownerOf: () => owner,
    ackMs: 5,
    schedule: (fn, ms) => {
      const list = ms === 5 ? acks : timers;
      list.push(fn);
      return { cancel: () => { const i = list.indexOf(fn); if (i >= 0) list.splice(i, 1); } };
    }
  });
  return { r, sent, timers, acks };
}

describe('the shader fallback relay', () => {
  it('runs a request in the window the job works for and hears only that window', async () => {
    const { r, sent } = relay([1, 2, 3], 2);
    const answer = r.run(9, request(), [new Uint16Array(8), new Uint16Array(8)]);
    expect(sent.map((s) => s.window)).toEqual([2]);
    const { id, textures } = sent[0]!.job;
    expect(textures).toHaveLength(2);
    r.answer(1, { id, ok: true, rgba16f: new Uint16Array(8).fill(1) });
    r.answer(2, { id, ok: true, rgba16f: new Uint16Array(8).fill(7) });
    const result = await answer;
    expect(result.ok && Array.from(result.rgba16f)).toEqual(new Array(8).fill(7));
  });

  it('uses another window when the owner is gone, and fails named when none is open', async () => {
    const { r, sent } = relay([404, 5], 404);
    void r.run(9, request(), []);
    await flush();
    expect(sent.map((s) => s.window)).toEqual([5]);
    const none = await relay([], null).r.run(9, request(), []);
    expect(none).toEqual({ ok: false, reason: 'unavailable', message: expect.stringContaining('no window') });
  });

  it('passes failures on, times out, and fails what a closed window was asked', async () => {
    const { r, sent, timers } = relay([1], null);
    const refused = r.run(9, request(), []);
    r.answer(1, { id: sent[0]!.job.id, ok: false, reason: 'watchdog', message: 'its draw did not finish within 10 s' });
    expect(await refused).toEqual({ ok: false, reason: 'watchdog', message: 'its draw did not finish within 10 s' });
    const slow = r.run(9, request(), []);
    timers[0]!();
    expect(await slow).toMatchObject({ ok: false, reason: 'unavailable' });
    const closed = r.run(9, request(), []);
    r.forget(1);
    expect(await closed).toMatchObject({ ok: false, reason: 'unavailable', message: expect.stringContaining('closed') });
    expect(fallbackErrorCode('compile')).toBe('shaderCompile');
    expect(fallbackErrorCode('watchdog')).toBe('watchdogTimeout');
    expect(fallbackErrorCode('tooLarge')).toBe('budgetExceeded');
    expect(fallbackErrorCode('contextLost')).toBe('unsupported');
  });
});

describe('acknowledgements', () => {
  it('passes over a window that does not acknowledge a request, quickly, and gives one that does the full timeout', async () => {
    const { r, sent, timers, acks } = relay([1, 2], 1);
    const answer = r.run(9, request(), []);
    expect(sent.map((s) => s.window)).toEqual([1]);
    // Window 1 is reloading: no acknowledgement within ackMs.
    acks[0]!();
    await flush();
    expect(sent.map((s) => s.window)).toEqual([1, 2]);
    // A late answer from window 1 is not heard.
    r.answer(1, { id: sent[0]!.job.id, ok: true, rgba16f: new Uint16Array(8).fill(1) });
    // Window 2 acknowledges: its acknowledgement deadline is gone, the full timeout stays.
    r.answer(2, { id: sent[1]!.job.id, received: true });
    expect(acks).toHaveLength(0);
    expect(timers).toHaveLength(1);
    r.answer(2, { id: sent[1]!.job.id, ok: true, rgba16f: new Uint16Array(8).fill(3) });
    const result = await answer;
    expect(result.ok && Array.from(result.rgba16f)).toEqual(new Array(8).fill(3));
    expect(timers).toHaveLength(0);
  });

  it('fails unavailable, naming it, when no window acknowledges', async () => {
    const { r, acks } = relay([1], null);
    const answer = r.run(9, request(), []);
    acks[0]!();
    expect(await answer).toEqual({ ok: false, reason: 'unavailable', message: expect.stringContaining('acknowledged') });
  });
});

describe('serving a job engine fallback request', () => {
  let dir = '';
  afterEach(async () => { if (dir) await rm(dir, { recursive: true, force: true }); });

  const body = (over: Partial<HostShaderFallback> = {}): HostShaderFallback => ({
    request: request(), area: 'ab'.repeat(16), textures: ['in-0.rgba16f', 'in-1.rgba16f'], output: 'out.rgba16f', ...over
  });
  const scratch = () => ({ kind: { kind: 'cache', cache: 'scratch' } as const, path: dir });

  it('reads the textures, runs the request, and writes the output', async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), 'pm-fallback-serve-'));
    await writeFile(path.join(dir, 'in-0.rgba16f'), Buffer.from(new Uint16Array(8).fill(0x3c00).buffer));
    await writeFile(path.join(dir, 'in-1.rgba16f'), Buffer.from(new Uint16Array(8).fill(0x3800).buffer));
    let seen: (Uint16Array | Float32Array)[] = [];
    await serveHostFallback(body(), scratch, async (_r, textures) => {
      seen = textures;
      return { ok: true, rgba16f: new Uint16Array(8).fill(0x3400) };
    });
    expect(seen.map((t) => t[0])).toEqual([0x3c00, 0x3800]);
    const out = new Uint16Array(new Uint8Array(await readFile(path.join(dir, 'out.rgba16f'))).buffer);
    expect(Array.from(out)).toEqual(new Array(8).fill(0x3400));
  });

  it('keeps float32 input and output bits in job scratch files', async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), 'pm-fallback-serve32-'));
    const input = Float32Array.from([1 + 2 ** -20, -2.125, 0.0001, 1, 0.33333334, 2.75, -0.25, 0.5]);
    const bits = new Uint32Array(input.buffer).slice();
    await writeFile(path.join(dir, 'in-0.rgba32f'), new Uint8Array(input.buffer));
    const req = { ...request(1), colorDepth: 32 };
    await serveHostFallback(body({ request: req, textures: ['in-0.rgba32f'], output: 'out.rgba32f' }), scratch, async (_r, textures) => {
      expect(textures).toHaveLength(1);
      expect(textures[0]).toBeInstanceOf(Float32Array);
      expect(new Uint32Array(textures[0]!.buffer)).toEqual(bits);
      return { ok: true, rgba16f: new Uint16Array(0), rgba32f: input };
    });
    const output = new Uint8Array(await readFile(path.join(dir, 'out.rgba32f')));
    expect(new Uint32Array(output.buffer)).toEqual(bits);
  });

  it('refuses paths, other areas, short textures, and answers runner failures with their codes', async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), 'pm-fallback-serve-'));
    const ok = async () => ({ ok: true as const, rgba16f: new Uint16Array(8) });
    const code = (p: Promise<unknown>) => p.then(() => 'resolved', (e: unknown) => (e instanceof ProtocolError ? e.code : String(e)));
    expect(await code(serveHostFallback(body({ textures: ['../x', 'in-1'] }), scratch, ok))).toBe('invalidMessage');
    expect(await code(serveHostFallback(body({ textures: ['in-0.rgba16f'] }), scratch, ok))).toBe('invalidMessage');
    expect(await code(serveHostFallback(body(), () => ({ kind: { kind: 'cache', cache: 'thumbnails' }, path: dir }), ok))).toBe('capabilityRejected');
    await writeFile(path.join(dir, 'in-0.rgba16f'), Buffer.alloc(16));
    await writeFile(path.join(dir, 'in-1.rgba16f'), Buffer.alloc(4));
    expect(await code(serveHostFallback(body(), scratch, ok))).toBe('invalidMessage');
    await writeFile(path.join(dir, 'in-1.rgba16f'), Buffer.alloc(16));
    expect(await code(serveHostFallback(body(), scratch, null))).toBe('unsupported');
    expect(await code(serveHostFallback(body(), scratch, async () => ({ ok: false, reason: 'compile', message: 'no' })))).toBe('shaderCompile');
    expect(await code(serveHostFallback(body(), scratch, async () => ({ ok: false, reason: 'watchdog', message: 'no' })))).toBe('watchdogTimeout');
    // A transient failure keeps the runner's reason in its message.
    const lost = await serveHostFallback(body(), scratch, async () => ({ ok: false, reason: 'contextLost', message: 'lost' })).catch((e: unknown) => e);
    expect(lost).toMatchObject({ code: 'unsupported', message: 'contextLost: lost' });
    expect(await readdir(dir)).not.toContain('out.rgba16f');
  });
});
