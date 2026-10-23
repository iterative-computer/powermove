/*
 * Main's side of the job engine's decision 19 fallback runner (contract
 * 14, `Request::ShaderFallback`; amendment 21: an export draws what only
 * WebGL draws, it is never routed to the current exporter for it).
 *
 * The job engine writes the textures a program samples into its session's
 * scratch cache area and asks the host to run the request. Main reads them
 * (`NativeEngine`), asks a window's renderer to run the request in its
 * runner (`engine-connection/shader-fallback.ts`) over IPC, and writes the
 * pixels back. The window is the one the job works for (an export's or an
 * agent frame batch's owner); render queue jobs and jobs whose window
 * closed use any window with the new engine. The runner is stateless
 * (the request carries everything), so any window draws the same pixels.
 *
 * A window acknowledges a request as soon as its runner has it
 * (`{ id, received: true }`, sent by the preload's listener). A window that
 * does not within `ackMs` (it is reloading, or its renderer has not
 * installed the runner yet) is passed over for the next one, so a job does
 * not wait the full timeout on a window that never heard it.
 */
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import type { CapabilityKind, HostShaderFallback, ShaderFallbackRequest } from '../../shared/engine-protocol';
import { isHandle, isSafeUint, ProtocolError } from './protocol';

/** A window's answer (`ENGINE_IPC.shaderFallbackResult`). */
export type ShaderFallbackResult =
  | { ok: true; rgba16f: Uint16Array; rgba32f?: Float32Array }
  | { ok: false; reason: string; message: string };

/** main → renderer (`ENGINE_IPC.shaderFallback`). */
export type ShaderFallbackJob = { id: number; request: ShaderFallbackRequest; textures: (Uint16Array | Float32Array)[] };

export type ShaderFallbackRelayOptions = {
  /** Posts a job to a window; false when the window is gone. */
  send(window: number, job: ShaderFallbackJob): boolean;
  /** Windows that can run requests (the new engine on), in order of preference. */
  windows(): number[];
  /** The window a job engine session works for, when known. */
  ownerOf(session: number): number | null;
  /** How long a window may take (ms). The job engine waits 120 s. */
  timeoutMs?: number;
  /** How long a window may take to acknowledge a request (ms). */
  ackMs?: number;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

type Timer = { cancel(): void };
type Waiting = { window: number; resolve(result: ShaderFallbackResult | 'notReady'): void; timer: Timer; ack: Timer | null };

/** Default acknowledgement deadline (ms). */
export const ACK_MS = 10_000;

export class ShaderFallbackRelay {
  private next = 1;
  private waiting = new Map<number, Waiting>();

  constructor(private readonly options: ShaderFallbackRelayOptions) {}

  /** Runs `request` in a window's runner: the job's owner first, then
   *  any other window, passing over windows that do not acknowledge it. */
  async run(session: number, request: ShaderFallbackRequest, textures: (Uint16Array | Float32Array)[]): Promise<ShaderFallbackResult> {
    const windows = this.options.windows();
    const owner = this.options.ownerOf(session);
    const order = owner !== null && windows.includes(owner) ? [owner, ...windows.filter((w) => w !== owner)] : windows;
    let passed = 0;
    for (const window of order) {
      const outcome = await this.ask(window, { id: this.next++, request, textures });
      if (outcome !== 'notReady') return outcome;
      passed++;
    }
    return {
      ok: false,
      reason: 'unavailable',
      message: passed > 0
        ? `no window with the new engine acknowledged the WebGL fallback request (${passed} asked)`
        : 'no window with the new engine is open to run the WebGL fallback runner'
    };
  }

  private schedule(fn: () => void, ms: number): Timer {
    const schedule = this.options.schedule ?? ((f, t) => {
      const handle = setTimeout(f, t);
      return { cancel: () => clearTimeout(handle) };
    });
    return schedule(fn, ms);
  }

  /** Asks one window; `notReady` when it is gone or does not acknowledge. */
  private ask(window: number, job: ShaderFallbackJob): Promise<ShaderFallbackResult | 'notReady'> {
    const { id } = job;
    return new Promise((resolve) => {
      const ms = this.options.timeoutMs ?? 110_000;
      const settle = (result: ShaderFallbackResult | 'notReady') => {
        const w = this.waiting.get(id);
        if (!w) return;
        this.waiting.delete(id);
        w.timer.cancel();
        w.ack?.cancel();
        resolve(result);
      };
      const timer = this.schedule(() => settle({ ok: false, reason: 'unavailable', message: `the window did not answer within ${Math.round(ms / 1000)} s` }), ms);
      const ack = this.schedule(() => settle('notReady'), this.options.ackMs ?? ACK_MS);
      this.waiting.set(id, { window, resolve: settle, timer, ack });
      if (!this.options.send(window, job)) settle('notReady');
    });
  }

  /** A window answered (only the window asked is heard). */
  answer(window: number, message: unknown): void {
    if (typeof message !== 'object' || message === null) return;
    const m = message as Record<string, unknown>;
    const id = Number(m['id']);
    const waiting = this.waiting.get(id);
    if (!waiting || waiting.window !== window) return;
    if (m['received'] === true) {
      // The window's runner has it: it now has the full timeout.
      waiting.ack?.cancel();
      waiting.ack = null;
      return;
    }
    if (m['ok'] === true && m['rgba16f'] instanceof Uint16Array && (m['rgba32f'] === undefined || m['rgba32f'] instanceof Float32Array)) {
      waiting.resolve({ ok: true, rgba16f: m['rgba16f'], ...(m['rgba32f'] instanceof Float32Array ? { rgba32f: m['rgba32f'] } : {}) });
    } else {
      waiting.resolve({ ok: false, reason: String(m['reason'] ?? 'unavailable'), message: String(m['message'] ?? '').slice(0, 1024) });
    }
  }

  /** A window closed: what it was asked fails at once. */
  forget(window: number): void {
    for (const w of [...this.waiting.values()]) {
      if (w.window !== window) continue;
      w.resolve({ ok: false, reason: 'unavailable', message: 'the window running the fallback closed' });
    }
  }
}

/** A plain file name in a cache area (`pm_protocol::shader_fallback::file_name_ok`). */
export function fallbackFileNameOk(name: unknown): name is string {
  return typeof name === 'string' && name.length > 0 && name.length <= 128 && !name.startsWith('.') && /^[A-Za-z0-9._-]+$/.test(name);
}

/** The host's error code for a runner failure. */
export function fallbackErrorCode(reason: string): 'shaderCompile' | 'watchdogTimeout' | 'budgetExceeded' | 'unsupported' {
  // `tooLarge` (a target past the GPU's WebGL texture limit) is final like
  // `compile`; the job engine reads `budgetExceeded` back as it.
  return reason === 'compile' ? 'shaderCompile' : reason === 'watchdog' ? 'watchdogTimeout' : reason === 'tooLarge' ? 'budgetExceeded' : 'unsupported';
}

/**
 * Serves one `shaderFallback` request: checks the names, reads the
 * textures from the scratch area `resolveArea` names, runs the request
 * (`run`; `null`: no runner here), and writes the output file. Throws a
 * `ProtocolError` the job engine reads as the runner's failure.
 */
export async function serveHostFallback(
  body: HostShaderFallback,
  resolveArea: (handle: string) => { kind: CapabilityKind; path: string },
  run: ((request: ShaderFallbackRequest, textures: (Uint16Array | Float32Array)[]) => Promise<ShaderFallbackResult>) | null
): Promise<void> {
  const { request } = body;
  if (!isHandle(body.area) || !Array.isArray(body.textures) || !body.textures.every(fallbackFileNameOk) || !fallbackFileNameOk(body.output)
    || body.textures.length !== (request?.textures ?? 0) || !isSafeUint(request?.width) || !isSafeUint(request?.height)) {
    throw new ProtocolError('invalidMessage', 'a fallback request names plain files in its area, one per texture');
  }
  const area = resolveArea(body.area);
  if (area.kind.kind !== 'cache' || area.kind.cache !== 'scratch') throw new ProtocolError('capabilityRejected', 'fallback files live in the scratch cache area');
  const float32 = request.colorDepth === 32;
  const bytes = request.width * request.height * (float32 ? 16 : 8);
  const textures = await Promise.all(body.textures.map(async (name) => {
    const data = await readFile(path.join(area.path, name));
    if (data.byteLength !== bytes) throw new ProtocolError('invalidMessage', `texture ${name} has ${data.byteLength} bytes, not ${bytes}`);
    const values = float32 ? new Float32Array(bytes / 4) : new Uint16Array(bytes / 2);
    new Uint8Array(values.buffer).set(data);
    return values;
  }));
  if (!run) throw new ProtocolError('unsupported', 'No window can run the WebGL fallback runner here.');
  const result = await run(request, textures);
  if (!result.ok) {
    const code = fallbackErrorCode(result.reason);
    // The job engine reads `unsupported` as a transient failure; the
    // runner's own reason (`contextLost`, `deadline`, ...) stays in the message.
    throw new ProtocolError(code, code === 'unsupported' ? `${result.reason}: ${result.message || result.reason}` : result.message || result.reason);
  }
  const values = float32 ? result.rgba32f : result.rgba16f;
  if (!values || values.byteLength !== bytes || (float32 && result.rgba16f.byteLength !== 0) || (!float32 && result.rgba32f !== undefined)) {
    throw new ProtocolError('unsupported', 'the runner answered pixels of another size or format');
  }
  await writeFile(path.join(area.path, body.output), new Uint8Array(values.buffer, values.byteOffset, values.byteLength), { mode: 0o600 });
}
