// Renderer side of the S5 Electron harness. Runs in a sandboxed,
// context-isolated renderer with no preload; the CSP comes from the
// protocol handler in main.cjs.
import init, * as bindings from '../pkg/s5_wasm.js';
import { runAll } from '../js/bench.mjs';

const results_jsHeap = {};
const hex = (buf) => Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, '0')).join('');

window.s5 = {
  waiting: null,
  resumeStage: null,
  async init() {
    const env = {
      crossOriginIsolated: globalThis.crossOriginIsolated === true,
      userAgent: navigator.userAgent,
      hasNodeRequire: typeof globalThis.require !== 'undefined',
      hasProcess: typeof globalThis.process !== 'undefined',
    };
    try {
      const exports = await init({ module_or_path: new URL('../pkg/s5_wasm_bg.wasm', import.meta.url) });
      this.memory = exports.memory;
      env.wasm = 'ok';
    } catch (error) {
      env.wasm = 'failed';
      env.wasmError = `${error?.name}: ${error?.message}`;
    }
    // A string eval must stay blocked: the CSP adds wasm-unsafe-eval only.
    try { new Function('return 1')(); env.stringEval = 'allowed'; } catch (error) { env.stringEval = `blocked (${error?.name})`; }
    return env;
  },
  resume() {
    this.waiting = null;
    const r = this.resumeStage;
    this.resumeStage = null;
    r?.();
  },
  async run() {
    const { results } = await runAll({
      bindings,
      memory: this.memory,
      now: () => performance.now(),
      platform: { runtime: 'electron-renderer', userAgent: navigator.userAgent, crossOriginIsolated: globalThis.crossOriginIsolated === true },
      sha256: async (bytes) => hex(await crypto.subtle.digest('SHA-256', bytes)),
      gc: typeof globalThis.gc === 'function' ? () => globalThis.gc() : () => {},
      onStage: (name) => new Promise((resolve) => {
        results_jsHeap[name] = performance.memory ? { usedJSHeapSize: performance.memory.usedJSHeapSize, totalJSHeapSize: performance.memory.totalJSHeapSize } : null;
        this.resumeStage = resolve;
        this.waiting = name;
      }),
    });
    results.jsHeap = results_jsHeap;
    return results;
  },
};
