/*
 * Store extensions and generated scripts in a document host worker: each runs
 * in a contained process (contained.ts) of the runtime bundle
 * (sandbox-runtime.mjs), the headless counterpart of the editor's
 * opaque-origin frames. The kernel's own sandbox host (kernel/sandbox-host.ts)
 * drives a Store extension exactly as it drives a frame: the same RPC, the
 * same schemas, budgets, watchdog and permission checks; only the transport is
 * the process's IPC channel instead of a frame's MessagePort.
 */
import type { ChildProcess } from 'node:child_process';
import { existsSync, realpathSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import type { ExtensionRecord } from '../../shared/extensions';
import type { SandboxRuntimeOptions } from '../../renderer/src/kernel/sandbox-host';
import type { SandboxInit } from '../../renderer/sandbox/shim-api';
import { containment, spawnContained } from './contained';
import { WorkerExtensions } from './extensions';

type ExternalRuntime = NonNullable<SandboxRuntimeOptions['process']>;

/** Where the runtime bundle is: POWERMOVE_SANDBOX_RUNTIME (the pool sets it), else next to this bundle. */
export function sandboxRuntimePath(): string | null {
  return process.env['POWERMOVE_SANDBOX_RUNTIME'] || null;
}

/**
 * What the runtime bundle reads besides itself: the one package it leaves
 * external (`ws`, which happy-dom imports), at the path Node resolves it by
 * and at its real path (package managers link it).
 */
export function runtimeReads(runtime: string): string[] {
  const reads = [path.dirname(runtime)];
  for (let dir = path.dirname(runtime); ; dir = path.dirname(dir)) {
    const candidate = path.join(dir, 'node_modules', 'ws');
    if (existsSync(candidate)) { reads.push(candidate, realpathSync(candidate)); break; }
    if (dir === path.dirname(dir)) break;
  }
  return reads;
}

/** Forwards a child's stderr to the worker log, bounded. */
function drain(child: ChildProcess, label: string, log: (line: string) => void): void {
  let budget = 64 * 1024;
  child.stderr?.on('data', (chunk: Buffer) => {
    if (budget <= 0) return;
    const text = chunk.toString('utf8').slice(0, budget);
    budget -= text.length;
    for (const line of text.split('\n')) if (line.trim()) log(`[engine] ${label}: ${line}`);
  });
}

/**
 * The loader's `sandboxProcess` hook: one contained process per Store
 * extension activation. A missing runtime bundle or bundle file fails the
 * activation by name (the loader records `activation-error`), never runs the
 * extension uncontained.
 */
export function storeSandbox(runtime: string | null, log: (line: string) => void): (record: ExtensionRecord) => ExternalRuntime {
  log(`[engine] Store extensions run contained (${containment()})`);
  return (record) => {
    const bundle = WorkerExtensions.fileOf(record);
    if (!runtime || !bundle) {
      const reason = !runtime ? 'the contained runtime (sandbox-runtime.mjs) is not built' : 'its bundle could not be read from the host';
      return { loaded: Promise.reject(new Error(`"${record.manifest?.name ?? record.id}" cannot run contained: ${reason}.`)), start: () => {}, terminate: async () => {} };
    }
    const network = (record.manifest?.permissions ?? []).includes('network');
    const child = spawnContained({ script: runtime, args: ['--extension'], read: [...runtimeReads(runtime), bundle], network });
    drain(child, record.id, log);
    let port: MessagePort | null = null;
    let settle: { resolve(): void; reject(error: Error): void } | null = null;
    const loaded = new Promise<void>((resolve, reject) => { settle = { resolve, reject }; });
    void loaded.catch(() => {});
    child.on('message', (raw) => {
      const message = raw as { t?: string; m?: unknown };
      if (message?.t === 'ready') { settle?.resolve(); settle = null; return; }
      // Only RPC frames cross; anything else the process sends is ignored.
      if (message?.t === 'rpc' && port) port.postMessage(message.m);
    });
    child.on('exit', (code, signal) => {
      settle?.reject(new Error(`The contained runtime for "${record.id}" exited (${signal ?? code}) before it started.`));
      settle = null;
      port?.close();
      port = null;
    });
    child.on('error', (error) => { settle?.reject(error); settle = null; });
    return {
      loaded,
      start(kernelPort: MessagePort, init: SandboxInit) {
        port = kernelPort;
        kernelPort.addEventListener('message', (event) => { if (child.connected) child.send({ t: 'rpc', m: (event as MessageEvent).data }); });
        kernelPort.start();
        child.send({ t: 'init', init, bundle: pathToFileURL(bundle).href, network });
      },
      terminate: async () => {
        if (child.exitCode !== null || child.signalCode !== null) return;
        await new Promise<void>((resolve) => { child.once('exit', () => resolve()); child.kill('SIGKILL'); });
      },
    };
  };
}

export interface ScriptMessage { source: string; code: unknown; project: unknown; input: unknown; runId: string; runtime: number }
export interface ScriptResult { ok: boolean; result?: unknown; error?: string }

/** How long the contained process may take to start before the run fails. */
const START_MS = 10_000;

/**
 * `PM.Script.isolate`: one generated script run in a contained process with
 * no network, the frame's time limit, and its error texts.
 */
export function scriptRuntime(runtime: string | null, log: (line: string) => void): (message: ScriptMessage) => Promise<ScriptResult> {
  return (message) => new Promise<ScriptResult>((resolve, reject) => {
    if (!runtime) { reject(new Error('The isolated runtime is not built (sandbox-runtime.mjs)')); return; }
    const child = spawnContained({ script: runtime, args: ['--script'], read: runtimeReads(runtime), network: false });
    drain(child, 'script', log);
    let done = false;
    let limit: ReturnType<typeof setTimeout> | null = null;
    const finish = (result: ScriptResult | Error) => {
      if (done) return;
      done = true;
      clearTimeout(start);
      if (limit) clearTimeout(limit);
      child.kill('SIGKILL');
      if (result instanceof Error) reject(result); else resolve(result);
    };
    // The frame's outer timer: a runtime that never comes up did not respond.
    const start = setTimeout(() => finish(new Error('The isolated runtime did not respond')), START_MS);
    child.on('message', (raw) => {
      const reply = raw as { t?: string; data?: ScriptResult };
      if (reply?.t === 'ready') child.send({ t: 'script', message });
      else if (reply?.t === 'started') {
        clearTimeout(start);
        // The frame's worker timer, from the moment the script started.
        limit = setTimeout(() => finish({ ok: false, error: 'Script exceeded its time limit' }), message.runtime || 900);
      } else if (reply?.t === 'result') finish(reply.data ?? { ok: false, error: 'Script returned no result' });
    });
    child.on('exit', () => finish({ ok: false, error: 'Script failed inside the isolated runtime' }));
    child.on('error', (error) => finish(error));
  });
}
