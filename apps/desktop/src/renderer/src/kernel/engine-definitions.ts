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
 *   `PM.EngineDefinitions.applied(...)` and `.failure(...)`.
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

  constructor(private readonly PM: any) {}

  /** `GL.compileError(key)` for engine-rendered programs. */
  compileError(key: string): string | null {
    return this.errors.get(key) ?? null;
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
  }

  /** An isolated failure at a revision (`failure` event). Older reports for a layer than one already applied are ignored. */
  failure(report: FailureReport): void {
    const { error, revision } = report;
    const scope = error.scope;
    if (scope.kind === 'definition') return this.definitionError(error);
    if (scope.kind !== 'layer' && scope.kind !== 'effect') return;
    const layer = this.layer(scope.layer);
    if (!layer) return;
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
  failure(report: FailureReport): void;
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
      const session = new SessionDefinitions(publisher, (update) => {
        meta.pushed(update);
        return send(update);
      });
      sessions.push(session);
      return session;
    },
    applied: (applied) => meta.applied(applied),
    failure: (report) => meta.failure(report),
    dispose() {
      for (const s of sessions) s.dispose();
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
    PM.GL.compileError = (key: string) => definitions.meta.compileError(key) ?? legacy(key);
  }
  PM.EngineDefinitions = definitions;
  return definitions;
}
