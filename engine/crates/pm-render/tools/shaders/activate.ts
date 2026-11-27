/*
 * Captures an extension module's registrations (`api.effects.register`,
 * `api.transitions.register`, `api.layers.register`) for the shader gate.
 *
 * The module is bundled in the parent (parsing, no execution) and its
 * `activate` runs in a child `bun` process under `sandbox-exec` with network
 * access and every file write denied, inside a fresh `node:vm` context with
 * stubbed APIs and a 2 s timeout. Store releases are third-party code; the
 * sandbox keeps them from touching this Mac even if they escape the vm
 * context. The child prints the registrations as JSON.
 *
 * As a module: `activateModule(entry, project)`. As the child: `bun
 * activate.ts --child` with the bundled code on stdin.
 */
import { join } from 'node:path';
import vm from 'node:vm';

export interface Registered { kind: 'effect' | 'transition' | 'layer'; def: any }
export interface Activation { registered: Registered[]; notes: string[] }

const PROFILE = '(version 1)(allow default)(deny network*)(deny file-write* (subpath "/"))(allow file-write* (literal "/dev/null"))';

async function child(): Promise<void> {
  const input = JSON.parse(await new Response(Bun.stdin.stream()).text()) as { code: string; filename: string; project: { w: number; h: number; fps: number } };
  const out: Registered[] = [];
  const notes: string[] = [];
  const plain = (value: unknown) => JSON.parse(JSON.stringify(value));
  const disposable = { dispose() {} };
  const noop: any = new Proxy(function () {}, { get: () => noop, apply: () => noop });
  const api: any = new Proxy({
    id: 'ext',
    effects: { register: (def: any) => { out.push({ kind: 'effect', def: plain(def) }); return disposable; }, list: () => [], get: () => undefined },
    transitions: { register: (def: any) => { out.push({ kind: 'transition', def: plain(def) }); return disposable; }, list: () => [], get: () => undefined },
    layers: { register: (def: any) => { out.push({ kind: 'layer', def: plain(def) }); return disposable; }, list: () => [], get: () => undefined },
    project: new Proxy({ get: async () => ({ ...input.project, layers: [] }) }, { get: (t: any, p) => t[p] ?? noop }),
    events: { on: () => disposable },
    onDispose: () => {},
    log: () => {},
  }, { get: (target: any, prop) => target[prop] ?? noop });
  const module = { exports: {} as any };
  const require = () => new Proxy({ default: noop }, { get: (t: any, p) => t[p] ?? noop });
  process.on('unhandledRejection', (error) => notes.push(`background task rejected: ${String(error).slice(0, 120)}`));
  vm.runInNewContext(input.code, {
    module, exports: module.exports, require, console: { log() {}, warn() {}, error() {} }, Promise, setTimeout, clearTimeout,
    document: noop, window: noop, navigator: noop, performance: noop, requestAnimationFrame: noop, cancelAnimationFrame: noop,
    OffscreenCanvas: noop, Image: noop, fetch: noop, addEventListener: noop, removeEventListener: noop,
  }, { filename: input.filename, timeout: 2000 });
  const activate = module.exports.default ?? module.exports.activate;
  if (typeof activate === 'function') {
    try {
      await Promise.race([activate(api), new Promise((_, reject) => setTimeout(() => reject(new Error('activation timed out')), 2000))]);
    } catch (error) {
      notes.push(`activation stopped after ${out.length} registrations (${String(error).slice(0, 120)})`);
    }
  }
  process.stdout.write(JSON.stringify({ registered: out, notes }));
}

/** Bundles `entry` and captures its registrations in the sandbox. */
export async function activateModule(entry: string, project = { w: 256, h: 144, fps: 30 }): Promise<Activation> {
  const built = await Bun.build({ entrypoints: [entry], format: 'cjs', target: 'browser', external: ['powermove', '*.svelte'] });
  if (!built.success) throw new Error(`build ${entry}: ${built.logs.join('\n')}`);
  const code = await built.outputs[0]!.text();
  const proc = Bun.spawn(['sandbox-exec', '-p', PROFILE, process.execPath, join(import.meta.dir, 'activate.ts'), '--child'], {
    stdin: new TextEncoder().encode(JSON.stringify({ code, filename: entry, project })),
    stdout: 'pipe', stderr: 'pipe', env: { PATH: process.env.PATH ?? '', HOME: process.env.HOME ?? '', BUN_RUNTIME_TRANSPILER_CACHE_PATH: '0' },
  });
  const timer = setTimeout(() => proc.kill(), 20_000);
  const [stdout, stderr, status] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]);
  clearTimeout(timer);
  if (status !== 0) throw new Error(`sandboxed activation of ${entry} failed (${status}): ${stderr.slice(0, 400)}`);
  return JSON.parse(stdout);
}

if (process.argv.includes('--child')) await child();
