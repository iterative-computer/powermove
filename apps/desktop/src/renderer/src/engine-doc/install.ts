/*
 * Installs the document mirror (pm-wasm), the write barrier, and the native
 * evaluator into the legacy registry (docs/rust-engine.md, section 6; section
 * 26, Phase 1).
 *
 * With the flag off (the default) only `PM.doc` exists, as no-ops, so writers
 * can call it unconditionally. With the flag on:
 *
 * - `PM.proj` becomes an accessor. Assigning another document resyncs the
 *   mirror (another project) or diffs it whole (another version of the same
 *   project); in tracking mode reads hand out the write-tracking Proxy.
 * - `PM.touch(paths?, canonical?)` records paths; without paths it asks for a
 *   structural diff of the open composition (production) or relies on the
 *   Proxy (tracking mode).
 * - `PM.doc.write(paths, fn)` and `PM.doc.touched(paths)` are the writers'
 *   barrier calls; history commits, undo, and redo hand over their patch paths.
 * - `PM.EngineDoc.api` answers the kernel's synchronous queries (kernel/
 *   install.ts and kernel/editor-helpers.ts route through it).
 * - The production consistency check runs before saves and exports and on
 *   idle; the auditor and dual answers run in the test modes.
 */
import type { PathPart } from '../../../shared/patch';
import type { PMRegistry } from '../legacy/registry';
import { DocumentBarrier, type ConsistencyResult, type PatchBatch } from './barrier';
import { ShadowAuditor } from './auditor';
import { DualAnswers } from './dual';
import { OFF, rendererMode, type EngineDocMode } from './flags';
import { NativeApi } from './native';
import { createTracker, patchCloning, type Tracker } from './proxy';
import { hostSink, Reporter, type Sink } from './report';
import { compileFrom, instantiate, MODULE_PATH, type PmWasm } from './wasm';
import { FontLoader, type FontProvider } from './fonts';
import { setEvaluationWriteHook } from '../legacy/core/temporal-bridge';
import { inversePlane, is3DLayer, local3D, parent3D, perspectiveAmount, planeContains, planeMatrix, projectPoint, world3D } from '../legacy/core/space-3d';
import { groupMatrix, pathValues } from '../legacy/core/vector-paths';
import { evaluatedValue } from '../legacy/core/content-properties';
import { validMatteSource } from '../legacy/core/matte';
import { countTextUnits } from '../legacy/core/text-animation';
import { expressionDiagnostic } from '../legacy/core/expression';

export interface EngineDocOptions {
  mode?: EngineDocMode;
  /** Where the module comes from (default: fetched from the editor's origin). */
  loadModule?: () => Promise<WebAssembly.Module>;
  sink?: Sink;
  /** Audit interval in ms (test modes). */
  auditMs?: number;
  /** The headless document host: no renderer, so 2D bounds and picks
   *  (`PM.GL`) come from pm-wasm alone. */
  headless?: boolean;
  /** Registry answers and subsets for text layout (engine-doc/fonts.ts). The
   *  editor uses `window.powermove.engineFonts` when the preload exposes it. */
  fonts?: FontProvider | null;
}

export interface DocApi {
  /** Runs `fn` as a known writer of `paths` (relative to `PM.proj`, or canonical). */
  write<T>(paths: readonly PathPart[][], fn?: () => T, options?: { canonical?: boolean; origin?: 'edit' | 'undo' | 'redo' | 'metadata' }): T | undefined;
  /** Records paths written already. */
  touched(paths: readonly PathPart[][], options?: { canonical?: boolean; origin?: 'edit' | 'undo' | 'redo' | 'metadata' }): void;
  /** Flushes pending writes into the mirror now. */
  flush(): void;
}

export interface EngineDoc {
  mode: EngineDocMode;
  enabled: boolean;
  ready: boolean;
  api: NativeApi | null;
  barrier: DocumentBarrier | null;
  wasm: PmWasm | null;
  auditor: ShadowAuditor | null;
  dual: DualAnswers | null;
  tracker: Tracker | null;
  /** Resolves when the module is loaded and the mirror synced (or failed). */
  whenReady: Promise<boolean>;
  check(reason: string): ConsistencyResult | null;
  beforeSave(reason?: string): void;
  beforeExport(reason?: string): void;
  subscribe(fn: (batch: PatchBatch) => void): () => void;
  stats(): Record<string, unknown>;
  summary(): void;
}

const noopDoc: DocApi = {
  write: (_paths, fn) => fn?.(),
  touched: () => {},
  flush: () => {},
};

export function installEngineDoc(PM: PMRegistry, options: EngineDocOptions = {}): EngineDoc {
  const mode = options.mode ?? safeMode();
  if (!mode.evaluator) {
    PM.doc = noopDoc;
    const off: EngineDoc = {
      mode: OFF, enabled: false, ready: false, api: null, barrier: null, wasm: null, auditor: null, dual: null, tracker: null,
      whenReady: Promise.resolve(false),
      check: () => null, beforeSave: () => {}, beforeExport: () => {}, subscribe: () => () => {}, stats: () => ({ enabled: false }), summary: () => {},
    };
    PM.EngineDoc = off;
    return off;
  }

  const reporter = new Reporter(options.sink ?? hostSink());
  let wasm: PmWasm | null = null;
  let barrier: DocumentBarrier | null = null;
  let writerDepth = 0;
  let coveredDepth = 0;
  let replacing = false;
  let current: any = PM.proj;
  let pendingReplace: 'resync' | 'diff' | null = null;

  // ── tracking Proxy (development and test) ─────────────────────────────
  const tracker: Tracker | null = mode.proxy ? createTracker({
    mode: mode.proxy,
    inWriter: () => writerDepth > 0 || replacing || !!PM.hist?.busy?.() || !!barrier?.flushing,
    onWrite: (path) => barrier?.touched([path]),
  }) : null;
  if (tracker) patchCloning(tracker);
  const unwrap = <T,>(v: T): T => (tracker ? tracker.unwrap(v) : v);
  let wrappedRaw: any = null, wrappedProxy: any = null;

  Object.defineProperty(PM, 'proj', {
    configurable: true,
    enumerable: true,
    get() {
      if (!tracker || !current || typeof current !== 'object') return current;
      if (current !== wrappedRaw) { wrappedRaw = current; wrappedProxy = tracker.wrap(current); }
      return wrappedProxy;
    },
    set(next: any) {
      const raw = unwrap(next);
      const previous = current;
      current = raw;
      if (previous === raw) return;
      // Another project resyncs; another version of this one diffs whole.
      const sameProject = previous && raw && typeof previous === 'object' && typeof raw === 'object' && previous.id === raw.id;
      pendingReplace = sameProject && pendingReplace !== 'resync' ? 'diff' : 'resync';
      if (barrier) {
        if (pendingReplace === 'resync') barrier.markResync();
        else barrier.touchedCanonical([[]], 'load');
        pendingReplace = null;
      }
    },
  });

  // ── writers ───────────────────────────────────────────────────────────
  const record = (paths: readonly PathPart[][], options: { canonical?: boolean; origin?: any } = {}) => {
    if (!barrier || !paths.length) return;
    if (options.canonical) barrier.touchedCanonical(paths, options.origin ?? 'edit');
    else barrier.touched(paths, options.origin ?? 'edit');
  };
  const doc: DocApi = {
    write(paths, fn, options = {}) {
      writerDepth += 1;
      coveredDepth += 1;
      try { return fn?.(); }
      finally {
        writerDepth -= 1;
        coveredDepth -= 1;
        record(paths, options);
      }
    },
    touched: (paths, options) => record(paths, options),
    flush: () => barrier?.flush(),
  };
  PM.doc = doc;

  const legacyTouch = PM.touch;
  PM.touch = (paths?: PathPart[][], canonical?: boolean) => {
    if (Array.isArray(paths)) record(paths, { canonical });
    // A scoped transaction declared its paths with track() before writing.
    else if (barrier && !tracker && coveredDepth === 0 && !PM.hist?.scoped?.()) barrier.touchAll();
    return legacyTouch?.();
  };
  // Legacy evaluation adopts old keyframe handles into the live document
  // (temporal-bridge.ts): a write without a path.
  setEvaluationWriteHook(() => { if (barrier && !tracker) barrier.touchAll(); });

  // ── the native evaluator ──────────────────────────────────────────────
  const dual = mode.dual ? new DualAnswers(m => reporter.emit({ type: 'dual', ...m })) : null;
  const legacy = legacyImplementations(PM);
  if (options.headless && !PM.GL?.bounds) {
    // Nothing renders here: pm-wasm answers, and what it cannot (3D, fonts
    // the host never served) is null, as an unrenderable layer is.
    PM.GL = { ...(PM.GL || {}), headless: true,
      bounds: (L: any, T: any) => api.bounds(L, T),
      pick: (x: number, y: number, T: number, o?: any) => api.pick(x, y, T, o) };
  }
  const api = new NativeApi(PM, () => (state.ready ? wasm : null), () => barrier, unwrap, legacy, dual);
  const fontProvider = options.fonts !== undefined ? options.fonts : editorFonts();
  if (fontProvider) api.fonts = new FontLoader(fontProvider);
  api.onStale = (op, paths) => {
    reporter.emit({ type: 'audit', kind: 'stale-answer', revision: barrier?.revision ?? -1, paths: paths.slice(0, 20), detail: op });
    if (auditor) { auditor.stats.missedWrites += 1; auditor.stats.failures += 1; }
  };
  const auditor = mode.audit ? new ShadowAuditor(() => (state.ready ? barrier : null), f => reporter.emit({ type: 'audit', kind: f.kind, revision: f.revision, paths: f.paths, detail: f.detail })) : null;

  const onMismatch = (result: ConsistencyResult) => {
    reporter.emit({ type: 'consistency', reason: result.reason, revision: result.revision, missed: result.missed.slice(0, 20), ms: Math.round(result.ms * 1000) / 1000 });
    PM.bus?.emit?.('engine:consistency-mismatch', { reason: result.reason, revision: result.revision, missed: result.missed.length });
  };

  const subscribers = new Set<(batch: PatchBatch) => void>();
  let readyResolve: (ok: boolean) => void = () => {};
  const whenReady = new Promise<boolean>(resolve => { readyResolve = resolve; });

  const state: EngineDoc = {
    mode, enabled: true, ready: false, api, barrier: null, wasm: null, auditor, dual, tracker, whenReady,
    check(reason) {
      if (!state.ready || !barrier) return null;
      try { return barrier.check(reason); } catch (error) { console.warn('[engine-doc] consistency check failed to run', error); return null; }
    },
    beforeSave(reason = 'save') { state.check(reason); },
    beforeExport(reason = 'export') { state.check(reason); },
    subscribe(fn) { subscribers.add(fn); return () => subscribers.delete(fn); },
    stats() {
      return {
        mode: mode.name, ready: state.ready,
        barrier: barrier ? { revision: barrier.revision, ...barrier.stats } : null,
        native: api.stats,
        dual: dual ? { ...dual.total(), compared: dual.stats.compared, mismatched: dual.stats.mismatched, stale: dual.stats.stale } : null,
        audit: auditor ? { audits: auditor.stats.audits, failures: auditor.stats.failures, missedWrites: auditor.stats.missedWrites, mirrorMismatches: auditor.stats.mirrorMismatches, ms: Math.round(auditor.stats.ms) } : null,
        violations: tracker ? tracker.violations().slice(0, 50) : null,
        wasm: wasm && !wasm.dead ? { ...wasm.heap(), ...wasm.stats() } : null,
      };
    },
    summary() { reporter.emit({ type: 'summary', ...state.stats() }); },
  };
  PM.EngineDoc = state;

  // ── loading ───────────────────────────────────────────────────────────
  const loadModule = options.loadModule ?? (() => compileFrom(() => fetch(new URL(MODULE_PATH, (globalThis as any).document?.baseURI ?? (globalThis as any).location?.href).href)));
  const start = async (): Promise<boolean> => {
    try {
      wasm = await instantiate(await loadModule());
      barrier = new DocumentBarrier(wasm, () => current, { onMismatch });
      barrier.subscribe(batch => subscribers.forEach(fn => { try { fn(batch); } catch (error) { console.error(error); } }));
      api.reset();
      state.wasm = wasm;
      state.barrier = barrier;
      replacing = true;
      try { barrier.resync(); } finally { replacing = false; }
      pendingReplace = null;
      state.ready = true;
      reporter.emit({ type: 'ready', mode: mode.name, bytes: wasm.x.memory.buffer.byteLength });
      return true;
    } catch (error) {
      state.ready = false;
      reporter.emit({ type: 'error', message: String((error as Error)?.message ?? error) });
      console.warn('[engine-doc] pm-wasm unavailable; the legacy evaluator answers', error);
      return false;
    }
  };
  void start().then(ok => {
    readyResolve(ok);
    if (!ok) return;
    if (auditor) auditor.start(options.auditMs ?? 100);
    if (mode.audit || mode.dual) {
      const timer = setInterval(() => state.summary(), 10_000);
      (timer as any)?.unref?.();
    }
    scheduleIdleCheck();
  });

  // A trap leaves the instance dead: re-instantiate and resync.
  const revive = () => {
    if (!wasm?.dead || reviving) return;
    reviving = true;
    state.ready = false;
    reporter.emit({ type: 'dead' });
    void start().finally(() => { reviving = false; });
  };
  let reviving = false;
  const reviveTimer = setInterval(revive, 1000);
  (reviveTimer as any)?.unref?.();

  // The consistency check on idle: at most every 30 s, after writes.
  let lastCheckedRevision = -1;
  function scheduleIdleCheck() {
    const idle = (globalThis as any).requestIdleCallback as ((cb: () => void, o?: object) => number) | undefined;
    const run = () => {
      if (state.ready && barrier && barrier.revision !== lastCheckedRevision) {
        state.check('idle');
        lastCheckedRevision = barrier.revision;
      }
      setTimeout(() => (idle ? idle(run, { timeout: 30_000 }) : run()), 30_000);
    };
    setTimeout(() => (idle ? idle(run, { timeout: 30_000 }) : run()), 30_000);
  }

  return state;
}

/** The editor's font channel to main, when the preload exposes one
 *  (`engine-fonts:answer` and `engine-fonts:subset`, docs/engine/text.md). */
function editorFonts(): FontProvider | null {
  const bridge = (globalThis as any).powermove?.engineFonts;
  if (!bridge || typeof bridge.answer !== 'function' || typeof bridge.subset !== 'function') return null;
  const unwrapResult = (r: any) => { if (r && r.ok === false) throw new Error(r.error); return r && 'value' in r ? r.value : r; };
  return {
    sync: false,
    answer: async (queries: string) => unwrapResult(await bridge.answer(queries)),
    subset: async (blob: string, index: number) => new Uint8Array(unwrapResult(await bridge.subset(blob, index))),
  };
}

function safeMode(): EngineDocMode {
  try { return rendererMode(); } catch { return OFF; }
}

/** The legacy answers, read from the registry at call time. */
function legacyImplementations(PM: PMRegistry): Record<string, any> {
  return {
    ev: (...a: any[]) => PM.ev(...a),
    evP: (...a: any[]) => PM.evP(...a),
    active: (...a: any[]) => PM.active(...a),
    findProp: (...a: any[]) => PM.findProp(...a),
    allProps: (...a: any[]) => PM.allProps(...a),
    hasKeyAt: (...a: any[]) => PM.hasKeyAt(...a),
    wouldCycle: (...a: any[]) => PM.wouldCycle(...a),
    resolveContent: (...a: any[]) => PM.resolveContent(...a),
    localMatrix: (...a: any[]) => PM.localMatrix(...a),
    worldMatrix: (...a: any[]) => PM.worldMatrix(...a),
    transformParentMatrix: (...a: any[]) => PM.transformParentMatrix(...a),
    mul: (...a: any[]) => PM.mul(...a),
    groupBounds: (...a: any[]) => PM.groupBounds(...a),
    groupAncestors: (...a: any[]) => PM.groupAncestors(...a),
    groupSpan: (...a: any[]) => PM.groupSpan(...a),
    expandGroups: (...a: any[]) => PM.expandGroups(...a),
    transformRoots: (...a: any[]) => PM.transformRoots(...a),
    bounds: (...a: any[]) => (PM.GL?.headless ? null : PM.GL.bounds(...a)),
    pick: (...a: any[]) => (PM.GL?.headless ? null : PM.GL.pick(...a)),
    textLayout: (...a: any[]) => (PM.textCaretLayout ? PM.textCaretLayout(...a) : null),
    local3D: (L: any, T: number) => local3D(PM, L, T),
    parent3D: (L: any, T: number, ...rest: any[]) => (parent3D as any)(PM, L, T, ...rest),
    world3D: (L: any, T: number, positioning?: boolean) => world3D(PM, L, T, positioning),
    is3DLayer: (L: any) => is3DLayer(PM, L),
    perspectiveAmount: (L: any, T: number) => perspectiveAmount(PM, L, T),
    planeMatrix: (L: any, T: number, positioning?: boolean) => planeMatrix(PM, L, T, positioning),
    projectPoint: (m: any, p: any) => projectPoint(m, p),
    inversePlane: (m: any) => inversePlane(m),
    planeContains: (L: any, T: number, x: number, y: number, b: any) => planeContains(PM, L, T, x, y, b),
    pathValues: (L: any, path: any, time: number, prefix?: string) => (prefix === undefined ? pathValues(PM, L, path, time) : pathValues(PM, L, path, time, prefix)),
    groupMatrix: (L: any, path: any, time: number) => groupMatrix(PM, L, path, time),
    evaluatedValue: (L: any, value: any, time: number, path: string) => evaluatedValue(PM, L, value, time, path),
    validMatteSource: (layers: any[], layer: any, id: any) => validMatteSource(layers, layer, id),
    countTextUnits: (lines: any, unit: any) => countTextUnits(lines, unit),
    expressionDiagnostic: (source: string) => expressionDiagnostic(source),
  };
}
