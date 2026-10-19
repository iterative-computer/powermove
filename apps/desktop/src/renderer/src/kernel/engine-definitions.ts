/*
 * The kernel's side of contract 10 with "Use New Engine" on: the winning
 * effect, transition, and layer registrations go to the engines as
 * definition snapshots (`definition-snapshots.ts`), and what the engines
 * report back about them fills `uiState.getShaderMeta` and the compile
 * errors the shader UI shows (plan section 11, "Definition snapshots").
 *
 * - Main's relay (`nativeEngine.pushDefinitions`) receives the window's
 *   stream: a full update, then a delta per register, dispose, and hot
 *   reload. Main primes the sessions it opens itself (export and agent jobs)
 *   from it. An engine session the window opens subscribes its own stream
 *   with `PM.EngineDefinitions.session(send)`.
 * - `DefinitionsApplied` failures (per definition) and `FailureReport`
 *   events (per layer or effect, at a revision) are applied with
 *   `PM.EngineDefinitions.applied(...)` and `.failure(...)`. Layer reports
 *   hold for their revision: the engine repeats each that still holds at
 *   every revision it presents, before the frame, and `.frame(revision)`
 *   clears the ones a shown frame did not repeat.
 * - Compile errors follow the route: while the engine presents the
 *   composition, `GL.compileError` answers every engine key (`fx:`, `tr:`,
 *   `extension:`, `engine:`, and a shader layer's `sh:` key) from the
 *   engine's reports alone, so a program the engine compiles shows no error
 *   and an error the legacy compositor recorded before is never shown
 *   again; while the legacy compositor draws it (no viewer session, or the
 *   composition routed), its own record answers the keys it sets. A definition failure the engine reports
 *   for an older definitions generation than the session's latest push of
 *   that id is stale and ignored (the translation of a set already
 *   replaced).
 * - `settle()` lets pending extension reloads finish and sends the
 *   registries' pending changes now: a tool that renders after a
 *   publication calls it first, so main and the engines hold the
 *   publication before the render is asked for (no false "missing").
 *
 * With the flag off nothing is installed.
 */
import type { DefinitionsApplied, DefinitionsUpdate, EngineError, FailureReport } from '../../../shared/engine-protocol';
import { DefinitionPublisher, SessionDefinitions, originOf, type DefinitionRegistries, type Preambles } from './definition-snapshots';
import { recordFor } from './extensions.svelte';

/** The shader meta and compile errors the engines reported. */
export class EngineShaderMeta {
  /** Compile errors by legacy program key (`fx:<id>`, `tr:<id>`, `extension:<id>`, `engine:<layer>`). */
  private errors = new Map<string, string>();
  /** The revision each layer's meta came from. */
  private revisions = new Map<string, number>();
  /**
   * Layer reports in force, by layer: what they set and their revision. A
   * layer report holds for its revision; the engine repeats every report
   * that still holds at each revision it presents, before the frame, so one
   * older than a frame shown is cleared (`frame`).
   */
  private layerReports = new Map<string, { revision: number; kind: 'compile' | 'missing' | 'scene' }>();
  /**
   * Shader layers the watchdog turned off, with the code it judged: the
   * report holds until the layer's code changes (the engine does not repeat
   * it per revision; a changed code is another program).
   */
  private watchdogLayers = new Map<string, string>();
  /** The newest revision `frame` saw (layer meta is refreshed once per revision). */
  private shownRevision = -1;

  constructor(private readonly PM: any) {}

  /**
   * Who draws the open composition now: the engine (its viewer session
   * presents frames) or the legacy compositor (no session yet, a session
   * closed, or the engine routed the composition). Compile errors follow
   * the route: while the legacy compositor draws, its own record is current.
   */
  private route: 'engine' | 'legacy' = 'legacy';

  /** The viewer session's route (`DocumentSession.onRoute`); `pending` is not the engine's. */
  setRoute(route: string): void {
    this.route = route === 'engine' ? 'engine' : 'legacy';
  }

  /**
   * Whether the engine answers `GL.compileError(key)` (no legacy fallback).
   * `engine:<layer>` keys exist only in engine reports; the keys both
   * compositors use (`fx:`, `tr:`, `extension:`, a shader layer's `sh:`)
   * are the engine's while it presents and the legacy compositor's while
   * it draws, so a shader layer the legacy viewer draws shows the error the
   * legacy compile recorded, and one the engine draws never shows a stale
   * legacy error.
   */
  owns(key: unknown): boolean {
    if (typeof key !== 'string') return false;
    if (key.startsWith('engine:')) return true;
    return this.route === 'engine' && /^(fx|tr|extension|sh):/.test(key);
  }

  /** `GL.compileError(key)` for engine-rendered programs. A shader layer's
   *  legacy key (`sh:<layer>:<hash>`) reads that layer's engine report. */
  compileError(key: string): string | null {
    const direct = this.errors.get(key);
    if (direct !== undefined) return direct;
    if (typeof key === 'string' && key.startsWith('sh:')) {
      const layer = key.slice(3, key.lastIndexOf(':') > 2 ? key.lastIndexOf(':') : undefined);
      return this.errors.get('engine:' + layer) ?? null;
    }
    return null;
  }

  /** A definition's failures after an update (`definitionsApplied`). */
  applied(applied: DefinitionsApplied): void {
    for (const error of applied.failures) this.definitionError(error);
  }

  /** Clears what the engines reported for a definition that was pushed again. */
  pushed(update: DefinitionsUpdate): void {
    const ids = [...update.upserts.map((d) => d.id), ...update.removals.map((r) => r.id)];
    if (update.full) this.errors.clear();
    for (const id of ids) for (const prefix of ['fx:', 'tr:', 'extension:']) this.errors.delete(prefix + id);
    // An extension layer whose definition arrives is no longer missing (the
    // engine reports it again at the same revision if it still is).
    const upserted = new Set(update.upserts.map((d) => d.id as string));
    for (const [id, report] of this.layerReports) {
      if (report.kind !== 'missing') continue;
      const layer = this.layer(id);
      if (!layer || upserted.has(layer.d?.definition)) this.clearLayer(id);
    }
  }

  /**
   * A frame at `revision` is shown: layer reports from older revisions that
   * the engine did not repeat no longer hold (the code compiles again, the
   * definition was found, the layer is gone).
   */
  frame(revision: number): void {
    for (const [id, report] of [...this.layerReports]) if (report.revision < revision) this.clearLayer(id);
    if (revision === this.shownRevision) return;
    this.shownRevision = revision;
    for (const [id, code] of [...this.watchdogLayers]) {
      const layer = this.layer(id);
      if (!layer || String(layer.d?.code ?? '') !== code) {
        this.watchdogLayers.delete(id);
        this.errors.delete('engine:' + id);
      }
    }
    this.refreshLayerMeta();
  }

  /**
   * The shader meta the legacy compositor set for every shader and
   * fragment extension layer it drew, kept by the engine for the layers it
   * draws without a report: the key `GL.compileError` answers for, the
   * registered definition, and whether it is missing. Layers with a report
   * in force keep what the report set; 3D, scene, and mesh layers keep
   * theirs.
   */
  private refreshLayerMeta(): void {
    const ui = this.PM.UIState;
    if (!ui?.getShaderMeta || !ui.setShaderMeta) return;
    for (const layer of this.layers()) {
      const id = layer?.id;
      if (!id || this.layerReports.has(id) || this.watchdogLayers.has(id)) continue;
      const meta = ui.getShaderMeta(layer) ?? {};
      if (layer.type === 'shader') {
        const shaderKey = 'engine:' + id;
        if (meta.shaderKey !== shaderKey) ui.setShaderMeta(layer, { shaderKey });
      } else if (layer.type === 'extension') {
        const definitionId = String(layer.d?.definition ?? '');
        const definition = this.PM.Kernel?.layerTypes?.get?.(definitionId) ?? null;
        if (definition && definition.renderer?.kind !== 'fragment') continue;
        const patch = { shaderKey: 'extension:' + definitionId, definition, missing: !definition, missingAsset: null };
        if (meta.shaderKey !== patch.shaderKey || meta.definition !== definition || meta.missing !== patch.missing) ui.setShaderMeta(layer, patch);
      }
    }
  }

  private clearLayer(id: string): void {
    const report = this.layerReports.get(id);
    if (!report) return;
    this.layerReports.delete(id);
    const layer = this.layer(id);
    if (report.kind === 'compile') {
      this.errors.delete('engine:' + id);
    } else if (layer) {
      this.PM.UIState?.setShaderMeta?.(layer, report.kind === 'missing' ? { missing: false, missingAsset: null } : { missing: false, sceneError: null });
    }
  }

  /**
   * An isolated failure at a revision (`failure` event). Older reports for a
   * layer than one already applied are ignored. A definition's compile
   * failure carries the definitions generation it was translated at; one
   * older than `stream`'s latest push of that definition is stale.
   */
  failure(report: FailureReport, stream?: { changedAt?(kind: string, id: string): number | undefined } | null): void {
    const { error, revision } = report;
    const scope = error.scope;
    if (scope.kind === 'definition') {
      if (stream?.changedAt && (error.code === 'shaderCompile' || error.code === 'notHeadlessRenderable')) {
        const at = Math.max(...['effect', 'transition', 'layer'].map((kind) => stream.changedAt!(kind, scope.definition) ?? 0));
        if (revision < at) return;
      }
      return this.definitionError(error);
    }
    if (scope.kind !== 'layer' && scope.kind !== 'effect') return;
    const layer = this.layer(scope.layer);
    if (!layer) return;
    if (scope.kind === 'layer' && error.code === 'watchdogTimeout') {
      // The watchdog turned a shader layer off: shown as its compile error
      // until its code changes.
      const shaderKey = 'engine:' + scope.layer;
      this.watchdogLayers.set(scope.layer, String(layer.d?.code ?? ''));
      this.errors.set(shaderKey, describe(error));
      this.PM.UIState?.setShaderMeta?.(layer, { shaderKey, missing: false });
      return;
    }
    const last = this.revisions.get(scope.layer);
    if (last !== undefined && revision < last) return;
    this.revisions.set(scope.layer, revision);
    const text = describe(error);
    if (scope.kind === 'effect') {
      // A watchdog overrun or a failed effect: kept per effect definition.
      this.errors.set('fx:' + scope.definition, text);
      return;
    }
    const definition = layer.d?.definition ?? null;
    const kind = error.code === 'shaderCompile' || error.code === 'notHeadlessRenderable' ? 'compile'
      : error.code === 'definitionMissing' || error.code === 'mediaOffline' ? 'missing' : 'scene';
    // A report of another kind for the layer replaces the one in force.
    const previous = this.layerReports.get(scope.layer);
    if (previous && previous.kind !== kind) this.clearLayer(scope.layer);
    this.layerReports.set(scope.layer, { revision, kind });
    if (error.code === 'shaderCompile' || error.code === 'notHeadlessRenderable') {
      const shaderKey = 'engine:' + scope.layer;
      this.errors.set(shaderKey, text);
      this.PM.UIState?.setShaderMeta?.(layer, { shaderKey, definition, missing: false });
    } else if (error.code === 'definitionMissing') {
      this.PM.UIState?.setShaderMeta?.(layer, { definition, missing: true, missingAsset: null });
    } else if (error.code === 'mediaOffline') {
      const asset = error.detail?.kind === 'offline' ? error.detail.assets[0] ?? null : null;
      this.PM.UIState?.setShaderMeta?.(layer, { definition, missing: true, missingAsset: asset });
    } else {
      this.PM.UIState?.setShaderMeta?.(layer, { definition, missing: true, sceneError: text });
    }
  }

  private definitionError(error: EngineError): void {
    if (error.scope.kind !== 'definition') return;
    const id = error.scope.definition;
    const text = describe(error);
    this.errors.set('fx:' + id, text);
    this.errors.set('tr:' + id, text);
    this.errors.set('extension:' + id, text);
    // Extension layers drawn from this definition show it.
    for (const layer of this.layers()) {
      if (layer?.type === 'extension' && layer.d?.definition === id) {
        this.PM.UIState?.setShaderMeta?.(layer, { shaderKey: 'extension:' + id, definition: id, missing: false });
      }
    }
  }

  private layers(): any[] {
    const project = this.PM.proj;
    const out: any[] = [...(project?.layers ?? [])];
    for (const comp of Object.values(project?.comps ?? {}) as any[]) out.push(...(comp?.layers ?? []));
    return out;
  }

  private layer(id: string): any {
    return this.layers().find((layer) => layer?.id === id);
  }
}

/** One error as the shader UI shows it: the message, then each diagnostic. */
export function describe(error: EngineError): string {
  const lines = [error.message];
  if (error.detail?.kind === 'shaderCompile') {
    for (const d of error.detail.diagnostics) lines.push(d.line > 0 ? `line ${d.line}: ${d.message}` : `(generated code): ${d.message}`);
  }
  return lines.join('\n');
}

export interface EngineDefinitions {
  publisher: DefinitionPublisher;
  meta: EngineShaderMeta;
  /** A stream for one engine session: full first, then deltas. */
  session(send: (update: DefinitionsUpdate) => boolean | void): SessionDefinitions;
  applied(applied: DefinitionsApplied): void;
  /** A failure event of the session whose stream is `stream` (stale definition failures are ignored). */
  failure(report: FailureReport, stream?: { changedAt?(kind: string, id: string): number | undefined } | null): void;
  /**
   * Lets queued extension reloads finish and sends pending registry
   * changes to every stream now (main's relay and the engine sessions), so
   * a render asked for after it sees every publication.
   */
  settle(): Promise<void>;
  /** A frame at this revision is shown (clears layer reports it did not repeat). */
  frame(revision: number): void;
  /** The open session streams. */
  sessionCount(): number;
  dispose(): void;
}

type NativeEngine = { status(): Promise<{ enabled: boolean }>; pushDefinitions(update: DefinitionsUpdate): void };

/** Builds the publisher on the kernel's registries with the installed preambles. */
export function createEngineDefinitions(PM: any, registries: DefinitionRegistries): EngineDefinitions {
  const preambles = (): Preambles => ({ effect: PM.GLSL_PRE, transition: PM.TRANSITION_PRE, layer: PM.SHADER_HEADER });
  const publisher = new DefinitionPublisher(registries, {
    origin: (ownerId) => originOf(recordFor(ownerId) as any),
    preambles,
    onError: (kind, id, error) => console.warn(`[definitions] ${kind} ${id} was not sent to the engine`, error),
  });
  const meta = new EngineShaderMeta(PM);
  const sessions: SessionDefinitions[] = [];
  return {
    publisher,
    meta,
    session(send) {
      const session: SessionDefinitions = new SessionDefinitions(publisher, (update) => {
        meta.pushed(update);
        return send(update);
      }, () => {
        // A closed session leaves the list (one per viewer session opened).
        const i = sessions.indexOf(session);
        if (i >= 0) sessions.splice(i, 1);
      });
      sessions.push(session);
      return session;
    },
    applied: (applied) => meta.applied(applied),
    failure: (report, stream) => meta.failure(report, stream),
    async settle() {
      try { await PM.Kernel?.loader?.whenIdle?.(); } catch { /* a failed reload still settles */ }
      publisher.flush();
    },
    frame: (revision) => meta.frame(revision),
    sessionCount: () => sessions.length,
    dispose() {
      for (const s of [...sessions]) s.dispose();
      publisher.dispose();
    },
  };
}

/**
 * Installs the definition stream when the new engine is on: main's relay
 * gets every update, and `GL.compileError` answers for engine-rendered
 * programs too.
 */
export async function installEngineDefinitions(PM: any, bridge: { nativeEngine?: NativeEngine } | null): Promise<EngineDefinitions | null> {
  const engine = bridge?.nativeEngine;
  if (!engine?.pushDefinitions || !engine.status) return null;
  const status = await engine.status().catch(() => null);
  if (!status?.enabled || !PM.Kernel) return null;
  const definitions = createEngineDefinitions(PM, PM.Kernel);
  definitions.session((update) => engine.pushDefinitions(update));
  const legacy = PM.GL?.compileError;
  if (PM.GL && typeof legacy === 'function') {
    // The engine owns its programs' errors: never the legacy compositor's
    // record of an older compile for the same key.
    PM.GL.compileError = (key: string) => definitions.meta.owns(key) ? definitions.meta.compileError(key) : legacy(key);
  }
  PM.EngineDefinitions = definitions;
  return definitions;
}
