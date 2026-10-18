/*
 * The editor's document chain without a browser: the same modules the
 * renderer installs (legacy/bootstrap.ts), in the same order, minus the GPU,
 * the media decoders, and the UI. One call boots one project's registry; a
 * worker process calls it once (docs/engine/document-host.md).
 *
 * Installed here and not in the old headless host: core/media (asset
 * records), gl/shaders and gl/transitions (the effect and transition
 * registries `add_effect` and `set_transition` read), the captions service
 * (`generate_captions`), the scripting catalog, and the extension loader
 * with the built-ins, so the kernel's registries hold what the desktop's hold.
 */
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { Window } from 'happy-dom';

import type { PowermoveBridge } from '../../shared/ipc';
import type { FontProvider } from '../../renderer/src/engine-doc/fonts';
import type { EngineDocMode } from '../../renderer/src/engine-doc/flags';
import { DocumentHostError } from './protocol';
import { sandboxRuntimePath, scriptRuntime, storeSandbox } from './sandbox';

/** A synchronous store over a map the worker seeds and persists itself. */
export interface ChainStore {
  get(key: string, fallback?: unknown): unknown;
  set(key: string, value: unknown): boolean;
  setAsync(key: string, value: unknown): Promise<boolean>;
  del(key: string): void;
  flush(): Promise<void>;
}

export function memoryStore(seed: Record<string, unknown> = {}): ChainStore {
  const values = new Map<string, unknown>(Object.entries(seed));
  const copy = (value: unknown) => JSON.parse(JSON.stringify(value ?? null));
  return {
    get: (key, fallback) => values.has(key) ? copy(values.get(key)) : fallback,
    set: (key, value) => { values.set(key, copy(value)); return true; },
    setAsync: (key, value) => { values.set(key, copy(value)); return Promise.resolve(true); },
    del: (key) => { values.delete(key); },
    flush: async () => {},
  };
}

let window: Window | null = null;

/** happy-dom as the globals the editor's modules read. Once per process. */
export function installDom(): Window {
  if (window) return window;
  window = new Window({ url: 'https://engine.powermove.local/' });
  const globals = globalThis as Record<string, unknown>;
  globals['window'] = window;
  for (const key of ['document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'matchMedia', 'localStorage', 'sessionStorage', 'Image', 'Blob', 'File', 'FileReader', 'DOMParser', 'XMLSerializer', 'MutationObserver', 'ResizeObserver', 'IntersectionObserver']) {
    const value = (window as unknown as Record<string, unknown>)[key];
    if (value !== undefined && globals[key] === undefined) globals[key] = typeof value === 'function' && !/^[A-Z]/.test(key) ? (value as (...a: unknown[]) => unknown).bind(window) : value;
  }
  return window;
}

/** Where the headless host finds pm-wasm: POWERMOVE_PM_WASM, else the
 *  renderer build next to this bundle (out/renderer/engine/pm_wasm.wasm). */
export function pmWasmPath(): string {
  return process.env['POWERMOVE_PM_WASM'] ?? fileURLToPath(new URL('../renderer/engine/pm_wasm.wasm', import.meta.url));
}

/** Where the headless host finds pm-bridge (the native font registry). */
export function pmBridgePath(): string {
  return process.env['POWERMOVE_PM_BRIDGE'] ?? fileURLToPath(new URL('../../../../engine/target/pm-bridge/pm_bridge.node', import.meta.url));
}

/**
 * The native font registry (pm-bridge) in this process. With `required`
 * (the new engine on) a missing or failing registry is a named failure; off,
 * it is loaded only with POWERMOVE_ENGINE_FONTS=1, as before.
 */
export function headlessFonts(log: (line: string) => void, required = false): FontProvider | null {
  if (!required && process.env['POWERMOVE_ENGINE_FONTS'] !== '1') return null;
  const addon = pmBridgePath();
  try {
    const { FontRegistry } = createRequire(import.meta.url)(addon) as { FontRegistry: new () => { answer(q: string): string; subset(blob: string, index: number): Uint8Array; whole?(blob: string, index: number): Uint8Array } };
    const registry = new FontRegistry();
    // Whole faces, outlines included, when the registry has them: answers
    // that need glyph ink (font-anchor bounds, text bounds, picks over text)
    // are then native here, as everything else is, instead of `{ink}`.
    const whole = typeof registry.whole === 'function' ? (blob: string, index: number) => registry.whole!(blob, index) : undefined;
    if (!whole && required) throw new DocumentHostError('font-service-unavailable', `The native font service (${addon}) has no whole faces (FontRegistry.whole), which text ink needs. Rebuild pm-bridge.`, true);
    return { sync: true, answer: (queries) => registry.answer(queries), subset: (blob, index) => registry.subset(blob, index), ...(whole ? { whole } : {}) };
  } catch (error) {
    const message = `The native font service is unavailable (${addon}): ${(error as Error).message}`;
    if (required) throw new DocumentHostError('font-service-unavailable', message, true);
    log(`[engine] ${message}`);
    return null;
  }
}

export interface EnginePolicy {
  /** "Use New Engine": pm-wasm and the font service are required. */
  newEngine: boolean;
  /** POWERMOVE_NATIVE_EVALUATOR (the development modes stay development-only). */
  evaluator?: string;
}

/**
 * Installs the document mirror and native evaluator. With the new engine on
 * a missing module, a module that fails to load, or a missing font service is
 * a named `DocumentHostError` (retry: true), never the legacy evaluator.
 */
export async function installHeadlessEngineDoc(PM: Record<string, any>, log: (line: string) => void, policy: EnginePolicy = { newEngine: false }): Promise<{ ready: boolean; mode: string; fonts: 'registry' | 'none' }> {
  const { installEngineDoc } = await import('../../renderer/src/engine-doc/install');
  const { parseMode, withNewEngine } = await import('../../renderer/src/engine-doc/flags');
  const mode: EngineDocMode = withNewEngine(parseMode(policy.evaluator ?? process.env['POWERMOVE_NATIVE_EVALUATOR'] ?? 'on'), policy.newEngine);
  const path = pmWasmPath();
  const present = existsSync(path);
  if (mode.evaluator && !present) {
    if (policy.newEngine) throw new DocumentHostError('pm-wasm-unavailable', `pm-wasm was not found at ${path}. Rebuild the app (bun run build:wasm) or set POWERMOVE_PM_WASM.`, true);
    log(`[engine] pm-wasm not found at ${path}; the legacy evaluator answers (Use New Engine is off)`);
  }
  const fonts = mode.evaluator && present ? headlessFonts(log, policy.newEngine) : null;
  const doc = installEngineDoc(PM as never, {
    mode: mode.evaluator && present ? mode : parseMode('off'),
    loadModule: () => WebAssembly.compile(readFileSync(path)),
    sink: (line) => log(`[engine] ${line}`),
    headless: true,
    fonts,
    strict: policy.newEngine,
  });
  const ready = await doc.whenReady;
  if (!ready && policy.newEngine) throw new DocumentHostError('pm-wasm-unavailable', `pm-wasm at ${path} failed to load.`, true);
  log(`[engine] document mirror: ${ready ? `pm-wasm (${mode.name})` : 'legacy evaluator'}`);
  return { ready, mode: mode.name, fonts: fonts ? 'registry' : 'none' };
}

export interface ChainOptions {
  /** The bridge the editor's modules read (`window.powermove`). */
  bridge: Partial<PowermoveBridge>;
  store: ChainStore;
  log: (line: string) => void;
  policy: EnginePolicy;
}

export interface Chain {
  PM: Record<string, any>;
  engine: { ready: boolean; mode: string; fonts: 'registry' | 'none' };
  /** Boots the extension loader and the built-ins against the current project. */
  bootExtensions(): Promise<{ active: string[]; failed: Array<{ id: string; error: string }> }>;
  loader(): import('../../renderer/src/kernel/loader').Loader | null;
}

/**
 * Boots one registry. The order is the renderer's (legacy/bootstrap.ts):
 * the kernel before anything registers into it, the mirror after history,
 * captions before the harness.
 */
export async function bootChain(options: ChainOptions): Promise<Chain> {
  installDom();
  // The editor's modules read the bridge through kernel/bridge.ts, which
  // looks at `globalThis` (the renderer's window); here the DOM's window is
  // a separate object, so both carry it.
  (globalThis as any).window.powermove = options.bridge;
  (globalThis as any).powermove = options.bridge;
  const PM: Record<string, any> = ((globalThis as any).window.PM = {});
  const [diag, util, kernel, uiState, memory, fonts, easing, model, selection, anim, history] = await Promise.all([
    import('../../renderer/src/legacy/core/diag'), import('../../renderer/src/legacy/core/util'), import('../../renderer/src/kernel/install'),
    import('../../renderer/src/legacy/core/ui-state'), import('../../renderer/src/legacy/core/memory'), import('../../renderer/src/legacy/core/fonts'),
    import('../../renderer/src/legacy/core/easing'), import('../../renderer/src/legacy/core/model'), import('../../renderer/src/legacy/core/selection'),
    import('../../renderer/src/legacy/core/anim'), import('../../renderer/src/legacy/core/history'),
  ]);
  const [library, projects, editing, capabilities, media, shaders, transitions, captions, workspace, harness, scripting, builtins] = await Promise.all([
    import('../../renderer/src/legacy/core/library'), import('../../renderer/src/legacy/core/projects'), import('../../renderer/src/legacy/core/editing'),
    import('../../renderer/src/legacy/core/capabilities'), import('../../renderer/src/legacy/core/media'), import('../../renderer/src/legacy/gl/shaders'),
    import('../../renderer/src/legacy/gl/transitions'), import('../../renderer/src/captions/install'), import('../../renderer/src/legacy/core/workspace'),
    import('../../renderer/src/legacy/assistant/harness'), import('../../renderer/src/legacy/core/scripting'), import('../../renderer/src/kernel/builtins'),
  ]);
  diag.install(PM as never); util.install(PM as never);
  PM['store'] = options.store;
  void kernel.installKernel(PM as never);
  for (const step of [uiState, memory, fonts, easing, model, selection, anim, history]) step.install(PM as never);
  const engine = await installHeadlessEngineDoc(PM, options.log, options.policy);
  for (const step of [library, projects, editing, capabilities, media, shaders, transitions]) step.install(PM as never);
  captions.installCaptions(PM as never);
  for (const step of [workspace, harness, scripting]) step.install(PM as never);
  // Scripts run in the editor's opaque-origin frame and worker; here the
  // same worker source runs in a contained process (sandbox.ts), with the
  // same guarantees (PM.Script.catalog()). happy-dom is no isolation boundary.
  const runtime = sandboxRuntimePath();
  PM['Script'].isolate = scriptRuntime(runtime, options.log);
  let loader: import('../../renderer/src/kernel/loader').Loader | null = null;
  return {
    PM,
    engine,
    loader: () => loader,
    async bootExtensions() {
      // Store extensions run in a contained process each (sandbox.ts), driven by the kernel's own sandbox host.
      loader = await kernel.bootExtensions(PM['Kernel'], builtins.BUILTIN_EXTENSIONS, { sandboxProcess: storeSandbox(runtime, options.log) });
      const records = loader.records();
      const failed = records.filter((r) => r.health && r.health.state !== 'ok')
        .map((r) => ({ id: r.id, error: 'error' in r.health! ? String((r.health as { error?: unknown }).error) : r.health!.state }));
      return { active: loader.activeIds(), failed };
    },
  };
}
