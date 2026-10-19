/*
 * The contained runtime (document-host/contained.ts starts it): the editor's
 * sandbox document without a browser, for a document host worker.
 *
 *   node sandbox-runtime.mjs --extension   a Store extension's runtime role (renderer/sandbox/boot.ts bootRuntime)
 *   node sandbox-runtime.mjs --script      one generated script run (legacy/core/scripting.ts WORKER_SOURCE)
 *
 * It speaks to its worker over the process's IPC channel only:
 *   → { t: 'ready' }                       DOM and runtime table installed
 *   ← { t: 'init', init, bundle, network } (extension) start activation
 *   ↔ { t: 'rpc', m }                      the sandbox RPC (shared/sandbox-rpc.ts), as on the frame's port
 *   ← { t: 'script', message }             (script) run once
 *   → { t: 'started' } / { t: 'result', data }
 *
 * Before any untrusted code runs, Node's built-in modules and every other
 * import are refused to it, the process members that reach native code are
 * removed, and fetch and WebSocket follow the sandbox CSP's connect-src.
 */
import { registerHooks } from 'node:module';
import { installDom } from './document-host/chain';

type Send = (message: unknown) => void;

/** Process members that reach native code or other processes, removed before untrusted code runs. */
const NATIVE_MEMBERS = ['binding', '_linkedBinding', 'dlopen', 'getBuiltinModule', 'kill', 'chdir', 'setuid', 'setgid', 'seteuid', 'setegid', 'setgroups', 'initgroups', '_debugProcess', '_kill', 'openStdin', 'loadEnvFile', 'permission'];

function scrubProcess(): void {
  const proc = process as unknown as Record<string, unknown>;
  for (const key of NATIVE_MEMBERS) {
    try { Object.defineProperty(proc, key, { value: undefined, writable: false, configurable: false }); } catch { /* not configurable: left */ }
  }
}

/** Every import the untrusted module (or anything it imports) asks for is refused: its bundle is self-contained, as under the frame's script-src. */
function refuseImports(untrusted: () => string | null): void {
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const from = untrusted();
      if (from && context.parentURL && context.parentURL.split('?')[0] === from.split('?')[0]) {
        const error = new Error(`Refused to load "${specifier}": sandboxed extensions run self-contained bundles.`) as Error & { code: string };
        error.code = 'ERR_ACCESS_DENIED';
        throw error;
      }
      return nextResolve(specifier, context);
    },
  });
}

/** connect-src: data: and blob: always; https: and wss: with the network permission. */
function connectPolicy(network: boolean, violation: (blocked: string) => void): void {
  const allowed = (raw: string): boolean => {
    let url: URL;
    try { url = new URL(raw); } catch { return false; }
    return url.protocol === 'data:' || url.protocol === 'blob:' || (network && (url.protocol === 'https:' || url.protocol === 'wss:'));
  };
  const urlOf = (input: unknown): string => typeof input === 'string' ? input : input instanceof URL ? input.href : String((input as { url?: unknown })?.url ?? input);
  const guard = (fetcher: typeof fetch | undefined) => fetcher && (async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
    const url = urlOf(input);
    if (!allowed(url)) { violation(url); throw new TypeError('Failed to fetch'); }
    return fetcher(input, init);
  });
  const g = globalThis as Record<string, any>;
  if (g['fetch']) g['fetch'] = guard(g['fetch']);
  if (g['window']?.fetch) g['window'].fetch = guard(g['window'].fetch.bind(g['window']));
  const NativeSocket = g['WebSocket'];
  if (NativeSocket) {
    g['WebSocket'] = class extends NativeSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        const href = urlOf(url);
        if (!allowed(href)) {
          violation(href);
          throw Object.assign(new Error(`Refused to connect to '${href}' because it violates the Content Security Policy.`), { name: 'SecurityError' });
        }
        super(url, protocols);
      }
    };
  }
  // XMLHttpRequest and EventSource are not offered (no Node equivalent under the same policy).
  for (const name of ['XMLHttpRequest', 'EventSource']) if (g['window']) g['window'][name] = undefined;
}

async function runExtension(send: Send): Promise<void> {
  const window = installDom() as unknown as Window & typeof globalThis;
  const { bootRuntime, installSandboxRuntime } = await import('../renderer/sandbox/boot');
  installSandboxRuntime();
  let bundle: string | null = null;
  refuseImports(() => bundle);
  const channel = new MessageChannel();
  channel.port2.addEventListener('message', (event) => send({ t: 'rpc', m: (event as MessageEvent).data }));
  channel.port2.start();
  // Errors the extension leaves uncaught reach its runtime-error reports, as window errors do in the frame.
  process.on('uncaughtException', (error) => window.dispatchEvent(Object.assign(new window.Event('error'), { error, message: String(error?.message ?? error) })));
  process.on('unhandledRejection', (reason) => window.dispatchEvent(Object.assign(new window.Event('unhandledrejection'), { reason })));
  let started = false;
  process.on('message', (raw) => {
    const message = raw as { t?: string; m?: unknown; init?: any; bundle?: string; network?: boolean };
    if (message?.t === 'rpc') { channel.port2.postMessage(message.m); return; }
    if (message?.t !== 'init' || started || typeof message.bundle !== 'string') return;
    started = true;
    bundle = message.bundle;
    connectPolicy(message.network === true, (blocked) => window.dispatchEvent(Object.assign(new window.Event('securitypolicyviolation'), { violatedDirective: 'connect-src', blockedURI: blocked })));
    scrubProcess();
    const url = message.bundle;
    void bootRuntime(message.init, channel.port1, () => import(/* @vite-ignore */ url));
  });
  send({ t: 'ready' });
}

function runScript(send: Send): void {
  process.on('message', (raw) => {
    const message = raw as { t?: string; message?: { source: string; code: unknown; project: unknown; input: unknown; runId: unknown } };
    if (message?.t !== 'script' || !message.message) return;
    const { source, ...data } = message.message;
    // The worker source as the frame runs it, with `self` its scope.
    const scope: { postMessage(data: unknown): void; onmessage: ((event: { data: unknown }) => unknown) | null } = {
      postMessage: (result) => { send({ t: 'result', data: result }); },
      onmessage: null,
    };
    new Function('self', String(source))(scope);
    scrubProcess();
    send({ t: 'started' });
    void scope.onmessage?.({ data });
  });
  send({ t: 'ready' });
}

if (typeof process.send === 'function') {
  const send: Send = process.send.bind(process) as Send;
  // The worker owns this process: when its channel closes, the run is over.
  process.on('disconnect', () => process.exit(0));
  if (process.argv.includes('--extension')) void runExtension(send);
  else if (process.argv.includes('--script')) runScript(send);
}
