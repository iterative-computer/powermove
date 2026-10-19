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
import { findPath, NativeApi } from './native';
import { NativeEdits } from './edits';
import { createTracker, patchCloning, querySite, type Tracker } from './proxy';
import { hostSink, Reporter, type Sink } from './report';
import { compileEditorModule, imports, instantiate, moduleCompileTiming, PmWasm } from './wasm';
import { FontLoader, type FontProvider } from './fonts';
import { fontCensusStats } from './ui-font-census';
import { bridge as hostBridge } from '../kernel/bridge';
import { setEvaluationWriter } from '../legacy/core/temporal-bridge';
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
  /** Test builds: replayed Canvas raster measurements give ink kernels a legacy reference. */
  textReference?: boolean;
  /** Registry answers and subsets for text layout (engine-doc/fonts.ts). The
   *  editor uses `window.powermove.engineFonts` when the preload exposes it. */
  fonts?: FontProvider | null;
  /**
   * "Use New Engine" on (docs/engine/write-path.md, "Named failures"): a
   * module that fails to load, an instance that traps twice on one
   * question, or a font service that stopped answering is a named failure
   * (`engine:failure` on the bus, a report line, `failures()`), never a
   * silent legacy answer. The flag off keeps today's behavior.
   */
  strict?: boolean;
}

/** A named engine failure ("Use New Engine" on). */
export interface EngineFailure {
  code: 'native-answer-unavailable' | 'pm-wasm-unavailable' | 'pm-wasm-trap' | 'pm-wasm-error' | 'font-service-unavailable';
  message: string;
  /** Whether `PM.EngineDoc.retry()` can recover. */
  retry: boolean;
  at: number;
}

export interface DocApi {
  /** Runs `fn` as a known writer of `paths` (relative to `PM.proj`, or canonical). */
  write<T>(paths: readonly PathPart[][], fn?: () => T, options?: { canonical?: boolean; origin?: 'edit' | 'undo' | 'redo' | 'metadata' }): T | undefined;
  /** Records paths written already. */
  touched(paths: readonly PathPart[][], options?: { canonical?: boolean; origin?: 'edit' | 'undo' | 'redo' | 'metadata' }): void;
  /** History's `track`: paths a scoped transaction is about to write, held until it ends. */
  track(paths: readonly PathPart[][]): void;
  /** Records a write to `object`, wherever it is in the document (its path
   *  is searched for; an object outside the document is not a document write). */
  wroteObject(object: unknown): void;
  /** Flushes pending writes into the mirror now. */
  flush(): void;
}

export interface EngineDoc {
  mode: EngineDocMode;
  enabled: boolean;
  ready: boolean;
  api: NativeApi | null;
  /** pm-edit's kernels for the edit pipeline (engine-doc/edits.ts). */
  edits: NativeEdits | null;
  barrier: DocumentBarrier | null;
  wasm: PmWasm | null;
  auditor: ShadowAuditor | null;
  dual: DualAnswers | null;
  tracker: Tracker | null;
  /** Resolves when the module is loaded and the mirror synced (or failed). */
  whenReady: Promise<boolean>;
  /** Named failures replace legacy answers (the "Use New Engine" setting). */
  strict: boolean;
  /**
   * Readiness once the load in flight settles: the boot load, a `retry()`, or
   * a revival after a trap. Resolves at once with `ready` when none is.
   */
  settled(): Promise<boolean>;
  check(reason: string): ConsistencyResult | null;
  beforeSave(reason?: string): void;
  beforeExport(reason?: string): void;
  /**
   * The canonical patch stream (contract 5) for engine sessions: every batch
   * pm-wasm applied, in order. Receivers start from `snapshot()` and apply
   * batches whose `baseRevision` equals their revision; `load` batches are
   * resyncs. The tracer forwards the same stream to the interactive engine.
   */
  subscribe(fn: (batch: PatchBatch) => void): () => void;
  /** `fn` runs when a write leaves something to flush where nothing was
   *  (the engine connection then flushes, so edits reach the engine soon). */
  onPending(fn: () => void): () => void;
  /**
   * While `wanted()` holds, a document announcement on the bus (`layers`,
   * `project`, `assets`, `comps`) flushes before its listeners run, so the
   * engine starts on an edit while the page redraws its panels (an edit's
   * listeners took 10 to 20 ms at 1,000 layers; docs/engine/perf.md).
   */
  flushBeforeListeners(wanted: () => boolean): () => void;
  /** The flushed canonical document and its revision and hash (null until ready). */
  snapshot(): ReturnType<DocumentBarrier['snapshot']> | null;
  stats(): Record<string, unknown>;
  summary(): void;
  /** Named failures so far (strict mode), newest last. */
  failures(): EngineFailure[];
  /** Loads the module again after a `pm-wasm-unavailable` failure; resolves with readiness. */
  retry(): Promise<boolean>;
  /**
   * Shows the newest named failure's notice again (with Retry where it can
   * help): a request that could not run because the document module is not
   * ready says so instead of disappearing. Strict mode only.
   */
  showFailure(): void;
  /**
   * `fn` runs each time the module becomes ready again after it was not: a
   * successful `retry()` after `whenReady` resolved false, or an asynchronous
   * revival after a trap. The extension loader activates what waited; the
   * editor finishes its first-frame setup.
   */
  onReady(fn: () => void): () => void;
  /** Loads the faces the open composition's text needs before anything asks (asynchronous font providers). */
  warmFonts(): Promise<void>;
}

const noopDoc: DocApi = {
  write: (_paths, fn) => fn?.(),
  touched: () => {},
  track: () => {},
  wroteObject: () => {},
  flush: () => {},
};

export function installEngineDoc(PM: PMRegistry, options: EngineDocOptions = {}): EngineDoc {
  const mode = options.mode ?? safeMode();
  if (!mode.evaluator) {
    PM.doc = noopDoc;
    const off: EngineDoc = {
      mode: OFF, enabled: false, ready: false, api: null, edits: null, barrier: null, wasm: null, auditor: null, dual: null, tracker: null,
      whenReady: Promise.resolve(false), strict: false, settled: () => Promise.resolve(false),
      check: () => null, beforeSave: () => {}, beforeExport: () => {}, subscribe: () => () => {}, onPending: () => () => {}, flushBeforeListeners: () => () => {}, snapshot: () => null, stats: () => ({ enabled: false }), summary: () => {},
      failures: () => [], showFailure: () => {}, retry: () => Promise.resolve(false), onReady: () => () => {}, warmFonts: () => Promise.resolve(),
    };
    PM.EngineDoc = off;
    return off;
  }

  const reporter = new Reporter(options.sink ?? hostSink());
  /** Tells this page's summaries apart from other launches' in one report file. */
  const session = Math.random().toString(36).slice(2, 10);
  let wasm: PmWasm | null = null;
  let barrier: DocumentBarrier | null = null;
  let writerDepth = 0;
  let coveredDepth = 0;
  let replacing = false;
  let current: any = PM.proj;
  let pendingReplace: 'resync' | 'diff' | null = null;
  /** No transaction diff has run yet in this synchronous run. */
  let runFresh = true;
  /** A plain transaction announced a write without paths since its last diff. */
  let transactionDirty = false;

  // ── tracking Proxy (development and test) ─────────────────────────────
  const tracker: Tracker | null = mode.proxy ? createTracker({
    mode: mode.proxy,
    inWriter: () => writerDepth > 0 || replacing || !!PM.hist?.busy?.() || !!barrier?.flushing,
    onWrite: (path) => barrier?.touched([path]),
    onViolation: (v) => reporter.emit({ type: 'violation', path: v.path, site: v.site, count: v.count }),
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
      // A new document is often built from reads of the current one
      // (`rotateProject({ ...PM.proj })`): its members may be proxies.
      const raw = tracker ? tracker.unwrapDeep(next) : next;
      const previous = current;
      current = raw;
      if (previous === raw) return;
      // Another project resyncs (the only `load` batch); another version of
      // this one diffs the whole document into an incremental batch, which
      // applies only at its base revision (contract 5, invariant 4).
      const sameProject = previous && raw && typeof previous === 'object' && typeof raw === 'object' && previous.id === raw.id;
      pendingReplace = sameProject && pendingReplace !== 'resync' ? 'diff' : 'resync';
      if (barrier) {
        if (pendingReplace === 'resync') barrier.markResync();
        else barrier.touchedCanonical([[]], 'structuralDiff');
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
      // Marked before as well as after: code inside `fn` may ask (a listener
      // of an event the writer emits), and its flush must see these paths.
      if (fn) record(paths, options);
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
    track(paths) {
      // A scoped transaction's declared paths stay held until it ends: its
      // writes keep landing there, so every announcement re-marks them.
      if (PM.hist?.scoped?.()) for (const path of paths) held.set(JSON.stringify(path), [...path]);
      record(paths);
    },
    wroteObject(object) {
      if (!barrier || !object || typeof object !== 'object') return;
      const path = findPath(current, unwrap(object), 16);
      if (path) barrier.touched([path]);
    },
    flush: () => barrier?.flush(),
  };
  PM.doc = doc;
  const held = new Map<string, PathPart[]>();

  /** A write without a path: inside a scoped transaction its held paths;
   *  inside a plain one, nothing until a query (the commit hands over every
   *  path it changed); otherwise a structural diff. The tracking Proxy does
   *  not replace it: writers announce writes made through references the
   *  Proxy never handed out (a new layer from `PM.mkLayer`, edited after insertion). */
  const unknownWrite = () => {
    if (!barrier || coveredDepth > 0) return;
    if (PM.hist?.scoped?.()) { if (held.size) barrier.touched([...held.values()]); return; }
    if (PM.hist?.busy?.()) { transactionDirty = true; return; }
    held.clear();
    barrier.touchAll();
  };
  /**
   * Before a flush inside an open transaction: its writes reach the barrier
   * at its commit, but code inside it may ask first (the pen tool between
   * `begin` and its first `touch()`, a split that groups and selects). The
   * first flush of each synchronous run, and the first after an announcement
   * without paths, diffs what the transaction may have written: a plain
   * transaction's open composition, a scoped one's tracked paths. Later
   * flushes in the same run cost nothing (a drag asks many times per event).
   */
  function transactionWrites(): void {
    if (!barrier || coveredDepth > 0 || !PM.hist?.busy?.()) return;
    if (!runFresh && !transactionDirty) return;
    transactionDirty = false;
    if (runFresh) { runFresh = false; queueMicrotask(() => { runFresh = true; }); }
    // Speculative marks for the flush that is starting: not announced (an
    // announcement would ask for another flush, which marks again: a
    // microtask loop for as long as the transaction stays open).
    if (PM.hist?.scoped?.()) { if (held.size) barrier.touched([...held.values()], 'edit', { announce: false }); }
    else barrier.touchAll({ announce: false });
  }
  const legacyTouch = PM.touch;
  PM.touch = (paths?: PathPart[][], canonical?: boolean) => {
    if (Array.isArray(paths)) {
      record(paths, { canonical });
      if (!PM.hist?.scoped?.()) held.clear();
    } else unknownWrite();
    return legacyTouch?.();
  };
  // Writers that skip touch() still announce on the bus (PM.addLayer,
  // PM.removeLayers, settings, asset changes): diff what they may have written.
  // Marked before any listener runs: the kernel turns these events into
  // `project:changed`, and the viewer asks from there (inside an Edit's scoped
  // transaction too, where the held paths must be marked again first).
  const announcing = new Set(['layers', 'project', 'assets', 'comps']);
  const bus = PM.bus;
  if (bus && typeof bus.emit === 'function') {
    // One wrapper per bus; a later install (a replaced registry in tests)
    // takes the announcement over.
    bus.__engineDocAnnounce = unknownWrite;
    bus.__engineDocEager = () => {
      if (!barrier?.pending || barrier.flushing || ![...eager].some(wanted => wanted())) return;
      try { barrier.flush(); } catch (error) { console.warn('[engine-doc] flush before listeners failed', error); }
    };
    if (!bus.emit.__engineDoc) {
      const emit = bus.emit;
      const wrapped = function (this: unknown, event: unknown, ...args: unknown[]) {
        if (announcing.has(event as string)) {
          bus.__engineDocAnnounce?.();
          bus.__engineDocEager?.();
        }
        return emit.call(this ?? bus, event, ...args);
      };
      (wrapped as any).__engineDoc = true;
      bus.emit = wrapped;
    }
  }
  // `PM.ProjectIndex.invalidate()` is the registry's "the layer structure
  // changed" signal: code that rewrites `layers` outside history must call it
  // or the index keeps answering with the old layers, and it often runs
  // before any bus announcement (an assignment, then a selection that
  // redraws the overlay, then `touch()`). It announces a write without paths.
  const index = PM.ProjectIndex;
  if (index && typeof index.invalidate === 'function') {
    index.__engineDocAnnounce = unknownWrite;
    if (!index.invalidate.__engineDoc) {
      const invalidate = index.invalidate;
      const wrapped = function (this: unknown, ...args: unknown[]) {
        index.__engineDocAnnounce?.();
        return invalidate.apply(this ?? index, args);
      };
      (wrapped as any).__engineDoc = true;
      index.invalidate = wrapped;
    }
  }
  // A canceled transaction leaves its writes in place without a commit.
  if (PM.hist && typeof PM.hist.cancel === 'function') {
    const cancel = PM.hist.cancel;
    PM.hist.cancel = (...args: unknown[]) => {
      const wasBusy = !!PM.hist.busy?.();
      try { return cancel.apply(PM.hist, args); }
      finally { held.clear(); if (wasBusy && barrier) barrier.touchAll(); }
    };
  }
  // Legacy evaluation adopts old keyframe handles into the live document
  // (temporal-bridge.ts): a known writer without a path.
  setEvaluationWriter({
    scope: fn => { writerDepth += 1; try { return fn(); } finally { writerDepth -= 1; } },
    // The keys are usually in a composition that is not open (a precomp's
    // layer evaluated for the open one): find them and name their path.
    // Keys outside the document (a detached layer) are not a document write.
    wrote: (keys) => doc.wroteObject(keys),
  });

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
  if (options.headless) for (const op of ['gl.bounds', 'gl.pick', 'glyphLayout']) api.noReference.add(op);
  // The glyph layout get_project_state reports for selected text (gl/raster.ts on the desktop).
  if (options.headless && !PM.textLayout) PM.textLayout = (L: any, T: number = PM.time) => api.glyphLayout(L, T);
  /* A trap leaves the instance dead. The compiled module is kept, so a fresh
     instance is made synchronously (the module is far below Chromium's
     synchronous-instantiation limit) and resynced from the shadow, and the
     question that trapped is asked once more on it: the answer stays native. */
  api.revive = () => reviveNow();
  api.onFailure = (code, message) => fail(code, message, code === 'font-service-unavailable');
  const fontProvider = options.fonts !== undefined ? options.fonts : editorFonts();
  if (fontProvider) {
    api.fonts = new FontLoader(fontProvider, () => {
      // Font cache changes affect text bounds, carets, picks and overlays
      // even when the document revision has not moved.
      PM.invalidate?.();
      PM.bus?.emit?.('fonts');
    });
    api.fonts.watchSeeds(() => (state.ready ? wasm : null));
  }
  api.onStale = (op, paths) => {
    reporter.emit({ type: 'audit', kind: 'stale-answer', revision: barrier?.revision ?? -1, paths: paths.slice(0, 20), detail: op, site: querySite(undefined, 8), tx: PM.hist?.busy?.() ? (PM.hist?.scoped?.() ? 'scoped' : 'plain') : 'none' });
    if (auditor) { auditor.stats.missedWrites += 1; auditor.stats.failures += 1; }
  };
  const edits = new NativeEdits(PM, () => (state.ready ? wasm : null), () => barrier, unwrap, dual, () => api.fonts);
  if (options.headless && !options.textReference) for (const kernel of ['captureFontAnchor', 'anchorReference']) edits.noReference.add(kernel);
  edits.revive = () => reviveNow();
  edits.onFailure = (code, message) => fail(code, message, code === 'font-service-unavailable');
  const auditor = mode.audit ? new ShadowAuditor(() => (state.ready ? barrier : null), f => reporter.emit({ type: 'audit', kind: f.kind, revision: f.revision, paths: f.paths, detail: f.detail })) : null;

  const onMismatch = (result: ConsistencyResult) => {
    reporter.emit({ type: 'consistency', reason: result.reason, revision: result.revision, missed: result.missed.slice(0, 20), ms: Math.round(result.ms * 1000) / 1000 });
    PM.bus?.emit?.('engine:consistency-mismatch', { reason: result.reason, revision: result.revision, missed: result.missed.length });
  };

  if (options.strict) {
    api.onFallback = edits.onFallback = (op, reason) => {
      const code = reason === 'fonts' || reason === 'ink' ? 'font-service-unavailable' : 'native-answer-unavailable';
      const message = `The new engine could not answer ${op} (${reason}).`;
      fail(code, message, code === 'font-service-unavailable' || reason === 'unavailable');
      const error = new Error(message);
      error.name = 'EngineFailure';
      throw error;
    };
  }
  const failures: EngineFailure[] = [];
  const failed = new Set<string>();
  /** Reports a named failure once per code and message (strict mode only). */
  function fail(code: EngineFailure['code'], message: string, retry: boolean): void {
    if (!options.strict) return;
    const key = `${code}\u0000${message}`;
    if (failed.has(key)) return;
    failed.add(key);
    const failure: EngineFailure = { code, message, retry, at: Date.now() };
    failures.push(failure);
    reporter.emit({ type: 'failure', code, message, retry });
    PM.bus?.emit?.('engine:failure', failure);
    notice(failure);
  }
  /** Visible, never silent (amendment 21): the editor's own toast, with
   *  Retry where reloading the module can help. One toast per code. */
  function notice({ code, message, retry }: EngineFailure): void {
    PM.toast?.(message, 8000, { error: true, key: `engine-failure-${code}`,
      ...(retry ? { action: { label: 'Retry', run: () => { void state.retry(); } } } : {}) });
  }
  /** Readiness came back: everything that waited for it runs. */
  function notifyReady(): void {
    for (const fn of [...readyListeners]) { try { fn(); } catch (error) { console.error(error); } }
  }

  const subscribers = new Set<(batch: PatchBatch) => void>();
  const pendingListeners = new Set<() => void>();
  const eager = new Set<() => boolean>();
  const readyListeners = new Set<() => void>();
  let readyResolve: (ok: boolean) => void = () => {};
  const whenReady = new Promise<boolean>(resolve => { readyResolve = resolve; });

  const startup = { startedAt: performance.now(), moduleMs: 0, instantiateMs: 0, mirrorMs: 0, readyMs: 0 };
  const state: EngineDoc = {
    mode, enabled: true, ready: false, api, edits, barrier: null, wasm: null, auditor, dual, tracker, whenReady,
    strict: !!options.strict,
    settled: () => loading ?? Promise.resolve(state.ready),
    check(reason) {
      if (!state.ready || !barrier) return null;
      try { return barrier.check(reason); } catch (error) { console.warn('[engine-doc] consistency check failed to run', error); return null; }
    },
    beforeSave(reason = 'save') { state.check(reason); },
    beforeExport(reason = 'export') { state.check(reason); },
    subscribe(fn) { subscribers.add(fn); return () => { subscribers.delete(fn); }; },
    onPending(fn) { pendingListeners.add(fn); return () => { pendingListeners.delete(fn); }; },
    flushBeforeListeners(wanted) { eager.add(wanted); return () => { eager.delete(wanted); }; },
    snapshot() { return state.ready && barrier ? barrier.snapshot() : null; },
    stats() {
      return {
        mode: mode.name, ready: state.ready,
        startup: { ...startup, ...moduleCompileTiming() },
        barrier: barrier ? { revision: barrier.revision, ...barrier.stats } : null,
        native: api.stats,
        fontCensus: typeof document === 'undefined' ? null : fontCensusStats(document),
        edits: edits.stats,
        dual: dual ? { ...dual.total(), compared: dual.stats.compared, mismatched: dual.stats.mismatched, stale: dual.stats.stale, allowed: dual.stats.allowed, fragile: dual.stats.fragile, fragileBudget: dual.fragileBudget() } : null,
        audit: auditor ? { audits: auditor.stats.audits, failures: auditor.stats.failures, missedWrites: auditor.stats.missedWrites, mirrorMismatches: auditor.stats.mirrorMismatches, ms: Math.round(auditor.stats.ms) } : null,
        violations: tracker ? tracker.violations().slice(0, 50) : null,
        wasm: wasm && !wasm.dead ? { ...wasm.heap(), ...wasm.stats() } : null,
      };
    },
    summary() { reporter.emit({ type: 'summary', session, ...state.stats() }); },
    failures: () => failures.slice(),
    showFailure() {
      if (!options.strict) return;
      const latest = [...failures].reverse().find(f => f.code === 'pm-wasm-unavailable') ?? failures[failures.length - 1];
      if (latest) notice(latest);
      else fail('pm-wasm-unavailable', "The new engine's document module is not ready.", true);
    },
    retry: () => {
      if (!api.fonts && options.fonts === undefined) { const provider = editorFonts(); if (provider) api.fonts = new FontLoader(provider); }
      api.fonts?.retry();
      failed.clear();
      if (state.ready) return Promise.resolve(true);
      return start().then((ok) => {
        if (ok) {
          afterReady();
          notifyReady();
        }
        return ok;
      });
    },
    onReady(fn) { readyListeners.add(fn); return () => { readyListeners.delete(fn); }; },
    warmFonts: () => (state.ready ? api.warmFonts() : Promise.resolve()),
  };
  PM.EngineDoc = state;

  // ── loading ───────────────────────────────────────────────────────────
  const loadModule = options.loadModule ?? compileEditorModule;
  let compiledModule: WebAssembly.Module | null = null;
  /** Puts a fresh instance in place and resyncs it (the barrier keeps its revisions). */
  const attach = (instance: PmWasm): void => {
    wasm = instance;
    barrier = new DocumentBarrier(wasm, () => current, {
      onMismatch, revision: barrier?.revision ?? 0,
      beforeFlush: transactionWrites,
      scoped: () => {
        const comp = PM.scope?.length ? unwrap(PM.curComp?.()) : null;
        return comp && typeof comp.id === 'string' && comp !== current ? comp.id : null;
      },
    });
    barrier.subscribe(batch => subscribers.forEach(fn => { try { fn(batch); } catch (error) { console.error(error); } }));
    barrier.onPending(() => pendingListeners.forEach(fn => { try { fn(); } catch (error) { console.error(error); } }));
    api.reset();
    state.wasm = wasm;
    state.barrier = barrier;
    replacing = true;
    try { barrier.resync(); } finally { replacing = false; }
    pendingReplace = null;
    api.fonts?.applySeeds(wasm);
    state.ready = true;
    reporter.emit({ type: 'ready', mode: mode.name, bytes: wasm.x.memory.buffer.byteLength });
  };
  /** The load in flight (see `settled`). */
  let loading: Promise<boolean> | null = null;
  const start = (): Promise<boolean> => {
    const attempt = load().finally(() => { if (loading === attempt) loading = null; });
    loading = attempt;
    return attempt;
  };
  const load = async (): Promise<boolean> => {
    try {
      const started = performance.now();
      compiledModule = await loadModule();
      const loaded = performance.now();
      startup.moduleMs = loaded - started;
      const instance = await instantiate(compiledModule);
      startup.instantiateMs = performance.now() - loaded;
      const attached = performance.now();
      attach(instance);
      startup.mirrorMs = performance.now() - attached;
      performance.mark?.('powermove:document-ready');
      startup.readyMs = performance.now() - started;
      return true;
    } catch (error) {
      state.ready = false;
      reporter.emit({ type: 'error', message: String((error as Error)?.message ?? error) });
      if (options.strict) fail('pm-wasm-unavailable', `The new engine's document module did not load: ${String((error as Error)?.message ?? error)}`, true);
      else console.warn('[engine-doc] pm-wasm unavailable; the legacy evaluator answers', error);
      return false;
    }
  };
  /** Synchronous revival after a trap; false when there is no module to instantiate. */
  function reviveNow(): boolean {
    if (!compiledModule || (wasm && !wasm.dead)) return !!wasm && !wasm.dead;
    try {
      reporter.emit({ type: 'dead' });
      attach(new PmWasm(new WebAssembly.Instance(compiledModule, imports)));
      return true;
    } catch (error) {
      state.ready = false;
      console.warn('[engine-doc] pm-wasm could not be re-instantiated', error);
      return false;
    }
  }
  let afterReadyDone = false;
  function afterReady(): void {
    if (afterReadyDone) return;
    afterReadyDone = true;
    if (auditor) auditor.start(options.auditMs ?? 100);
    if (mode.audit || mode.dual) {
      // A summary every 2 s while counts change, and one as the page goes,
      // so short-lived test windows report their totals too.
      let last = '';
      const flushSummary = () => {
        const stats = JSON.stringify(state.stats());
        if (stats === last) return;
        last = stats;
        state.summary();
      };
      const timer = setInterval(flushSummary, 2_000);
      (timer as any)?.unref?.();
      (globalThis as any).addEventListener?.('pagehide', flushSummary);
    }
    scheduleIdleCheck();
  }
  void start().then(ok => {
    readyResolve(ok);
    if (ok) afterReady();
  });

  // A trap outside a query (a flush) leaves the instance dead: re-instantiate and resync.
  const revive = () => {
    if (!wasm?.dead || reviving) return;
    reviving = true;
    state.ready = false;
    if (reviveNow()) { reviving = false; notifyReady(); return; }
    reporter.emit({ type: 'dead' });
    void start().then(ok => { if (ok) notifyReady(); }).finally(() => { reviving = false; });
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
  const bridge = (hostBridge() as any)?.engineFonts;
  if (!bridge || typeof bridge.answer !== 'function' || typeof bridge.subset !== 'function') return null;
  const unwrapResult = (r: any) => {
    if (!r || typeof r !== 'object') throw new Error('the font registry did not answer');
    if (r.ok === false) throw new Error(r.error);
    return r.value;
  };
  // Seeds describe this renderer regardless of how face bytes are served.
  const seedSource = {
    ...(typeof bridge.seeds === 'function' ? { seeds: async () => String(unwrapResult(await bridge.seeds())) } : {}),
    ...(typeof bridge.onSeedsChanged === 'function' ? { onSeedsChanged: (listener: (json: string) => void) => { bridge.onSeedsChanged(listener); } } : {}),
  };
  if (typeof bridge.answerSync === 'function' && typeof bridge.wholeSync === 'function') return {
    sync: true,
    answer: (queries) => unwrapResult(bridge.answerSync(queries)),
    subset: (blob, index) => new Uint8Array(unwrapResult(bridge.wholeSync(blob, index))),
    whole: (blob, index) => new Uint8Array(unwrapResult(bridge.wholeSync(blob, index))),
    ...seedSource,
  };
  return {
    sync: false,
    answer: async (queries: string) => unwrapResult(await bridge.answer(queries)),
    subset: async (blob: string, index: number) => new Uint8Array(unwrapResult(await bridge.subset(blob, index))),
    ...seedSource,
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
    // Only a host without a raster asks (its PM.textLayout is the native one): nothing to fall back to.
    glyphLayout: () => null,
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
