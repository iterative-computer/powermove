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
 */
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import type { CapabilityKind, HostShaderFallback, ShaderFallbackRequest } from '../../shared/engine-protocol';
import { isHandle, isSafeUint, ProtocolError } from './protocol';

/** A window's answer (`ENGINE_IPC.shaderFallbackResult`). */
export type ShaderFallbackResult =
  | { ok: true; rgba16f: Uint16Array }
  | { ok: false; reason: string; message: string };

/** main → renderer (`ENGINE_IPC.shaderFallback`). */
export type ShaderFallbackJob = { id: number; request: ShaderFallbackRequest; textures: Uint16Array[] };

export type ShaderFallbackRelayOptions = {
  /** Posts a job to a window; false when the window is gone. */
  send(window: number, job: ShaderFallbackJob): boolean;
  /** Windows that can run requests (the new engine on), in order of preference. */
  windows(): number[];
  /** The window a job engine session works for, when known. */
  ownerOf(session: number): number | null;
  /** How long a window may take (ms). The job engine waits 120 s. */
  timeoutMs?: number;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

type Waiting = { window: number; resolve(result: ShaderFallbackResult): void; timer: { cancel(): void } };

export class ShaderFallbackRelay {
  private next = 1;
  private waiting = new Map<number, Waiting>();

  constructor(private readonly options: ShaderFallbackRelayOptions) {}

  /** Runs `request` in a window's runner. */
  run(session: number, request: ShaderFallbackRequest, textures: Uint16Array[]): Promise<ShaderFallbackResult> {
    const windows = this.options.windows();
    const owner = this.options.ownerOf(session);
    const order = owner !== null && windows.includes(owner) ? [owner, ...windows.filter((w) => w !== owner)] : windows;
    const id = this.next++;
    for (const window of order) {
      const answered = new Promise<ShaderFallbackResult>((resolve) => {
        const ms = this.options.timeoutMs ?? 110_000;
        const schedule = this.options.schedule ?? ((fn, t) => {
          const handle = setTimeout(fn, t);
          return { cancel: () => clearTimeout(handle) };
        });
        const timer = schedule(() => {
          if (this.waiting.delete(id)) resolve({ ok: false, reason: 'unavailable', message: `the window did not answer within ${Math.round(ms / 1000)} s` });
        }, ms);
        this.waiting.set(id, { window, resolve, timer });
      });
      if (this.options.send(window, { id, request, textures })) return answered;
      this.waiting.get(id)?.timer.cancel();
      this.waiting.delete(id);
    }
    return Promise.resolve({ ok: false, reason: 'unavailable', message: 'no window with the new engine is open to run the WebGL fallback runner' });
  }

  /** A window answered (only the window asked is heard). */
  answer(window: number, message: unknown): void {
    if (typeof message !== 'object' || message === null) return;
    const m = message as Record<string, unknown>;
    const id = Number(m['id']);
    const waiting = this.waiting.get(id);
    if (!waiting || waiting.window !== window) return;
    this.waiting.delete(id);
    waiting.timer.cancel();
    if (m['ok'] === true && m['rgba16f'] instanceof Uint16Array) {
      waiting.resolve({ ok: true, rgba16f: m['rgba16f'] });
    } else {
      waiting.resolve({ ok: false, reason: String(m['reason'] ?? 'unavailable'), message: String(m['message'] ?? '').slice(0, 1024) });
    }
  }

  /** A window closed: what it was asked fails at once. */
  forget(window: number): void {
    for (const [id, w] of [...this.waiting]) {
      if (w.window !== window) continue;
      this.waiting.delete(id);
      w.timer.cancel();
      w.resolve({ ok: false, reason: 'unavailable', message: 'the window running the fallback closed' });
    }
  }
}

/** A plain file name in a cache area (`pm_protocol::shader_fallback::file_name_ok`). */
export function fallbackFileNameOk(name: unknown): name is string {
  return typeof name === 'string' && name.length > 0 && name.length <= 128 && !name.startsWith('.') && /^[A-Za-z0-9._-]+$/.test(name);
}

/** The host's error code for a runner failure. */
export function fallbackErrorCode(reason: string): 'shaderCompile' | 'watchdogTimeout' | 'unsupported' {
  return reason === 'compile' ? 'shaderCompile' : reason === 'watchdog' ? 'watchdogTimeout' : 'unsupported';
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
  run: ((request: ShaderFallbackRequest, textures: Uint16Array[]) => Promise<ShaderFallbackResult>) | null
): Promise<void> {
  const { request } = body;
  if (!isHandle(body.area) || !Array.isArray(body.textures) || !body.textures.every(fallbackFileNameOk) || !fallbackFileNameOk(body.output)
    || body.textures.length !== (request?.textures ?? 0) || !isSafeUint(request?.width) || !isSafeUint(request?.height)) {
    throw new ProtocolError('invalidMessage', 'a fallback request names plain files in its area, one per texture');
  }
  const area = resolveArea(body.area);
  if (area.kind.kind !== 'cache' || area.kind.cache !== 'scratch') throw new ProtocolError('capabilityRejected', 'fallback files live in the scratch cache area');
  const bytes = request.width * request.height * 8;
  const textures = await Promise.all(body.textures.map(async (name) => {
    const data = await readFile(path.join(area.path, name));
    if (data.byteLength !== bytes) throw new ProtocolError('invalidMessage', `texture ${name} has ${data.byteLength} bytes, not ${bytes}`);
    const values = new Uint16Array(bytes / 2);
    new Uint8Array(values.buffer).set(data);
    return values;
  }));
  if (!run) throw new ProtocolError('unsupported', 'No window can run the WebGL fallback runner here.');
  const result = await run(request, textures);
  if (!result.ok) throw new ProtocolError(fallbackErrorCode(result.reason), result.message || result.reason);
  if (result.rgba16f.length * 2 !== bytes) throw new ProtocolError('unsupported', 'the runner answered pixels of another size');
  await writeFile(path.join(area.path, body.output), new Uint8Array(result.rgba16f.buffer, result.rgba16f.byteOffset, result.rgba16f.byteLength), { mode: 0o600 });
}
