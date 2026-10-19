/*
 * Definition snapshots for engine sessions (contract 10, plan section 11,
 * "Definition snapshots"; behind "Use New Engine").
 *
 * The kernel stays the validator and the GLSL generator: every winning
 * effect, transition, and extension-layer registration becomes a
 * `DefinitionSnapshot` with its kernel-wrapped GLSL (`kernel/glsl.ts`,
 * `kernel/extension-layers.ts`, the preambles `gl/shaders.ts` and
 * `gl/transitions.ts` install), parameters without their presentation
 * fields, and the effect flags. UI-only fields (`label`, `group`, `ui`,
 * `advanced`, `options`, `unit`, widget hints, `icon`, `color`, `defaults`)
 * never reach the engine.
 *
 * `DefinitionPublisher` follows the registries: last registered wins,
 * disposing restores the previous registration, and every register, dispose,
 * and hot reload produces one change (coalesced per microtask). Each sink
 * (`SessionDefinitions`) numbers its own generations: a full update first,
 * then deltas; a sink that reports a gap starts again with a full update.
 */
import type { DefinitionKind, DefinitionOrigin, DefinitionRemoval, DefinitionSnapshot, DefinitionsUpdate, LayerRenderer, ParamSpec, WrappedGlsl } from '../../../shared/engine-protocol';
import type { EffectDefinition, EffectParamDefinition, ExtensionLayerDefinition, TransitionDefinition } from './api';
import { extensionLayerFragment } from './extension-layers';
import { toLegacyFx, toLegacyTransition } from './glsl';
import type { Registry } from './registry';

/** The preambles the kernel wraps with (`PM.GLSL_PRE`, `PM.TRANSITION_PRE`, `PM.SHADER_HEADER`). */
export interface Preambles {
  effect: string;
  transition: string;
  layer: string;
}

const encoder = new TextEncoder();

/** Lines before `index` in `source`. */
const linesBefore = (source: string, index: number): number => {
  let n = 0;
  for (let i = 0; i < index; i++) if (source.charCodeAt(i) === 10) n++;
  return n;
};

/** `body` sits at the end of `source`, followed only by `tail`. */
export function wrappedGlsl(source: string, body: string, tail = ''): WrappedGlsl {
  const start = source.length - tail.length - body.length;
  if (start < 0 || source.slice(start, start + body.length) !== body) throw new Error('wrapped GLSL does not end with the body');
  return { source, bodyLineOffset: linesBefore(source, start), bodyUtf16Len: body.length, bodyUtf8Len: encoder.encode(body).length };
}

/* legacy/core/util.ts hexDigits/hex2rgb: the value a color binds is read by
   channel position, unparsable channels are 0. The snapshot carries the same
   value as #rrggbb. */
const hexDigits = (hex: unknown): string => {
  const s = String(hex ?? '').replace('#', '');
  return s.length === 3 || s.length === 4 ? s.split('').map((c) => c + c).join('') : s;
};
export function normalizedColor(hex: unknown): string {
  const s = hexDigits(hex);
  return '#' + [0, 2, 4].map((i) => ((parseInt(s.slice(i, i + 2), 16) || 0) & 255).toString(16).padStart(2, '0')).join('');
}

const finite = (value: unknown, fallback: number): number => {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
};

/**
 * One parameter as the engine binds it. The compositor binds colors as
 * vec3, toggles as 0/1, and every other type as `Number(value) || 0`
 * (`setParam`), so any other `type` is a number. A number's range is
 * presentation only; it is made well-formed (finite, min ≤ max) so one
 * definition's slider bounds can never invalidate an update.
 */
export function paramSpec(param: EffectParamDefinition): ParamSpec {
  const key = String(param.k);
  if (param.type === 'color') return { type: 'color', key, default: normalizedColor(param.def) };
  if (param.type === 'toggle') return { type: 'toggle', key, default: !!param.def };
  const numeric = param as { def?: unknown; min?: unknown; max?: unknown; step?: unknown };
  const def = finite(numeric.def, 0);
  const a = finite(numeric.min, def), b = finite(numeric.max, def);
  const step = Number(numeric.step);
  return { type: 'number', key, default: def, min: Math.min(a, b), max: Math.max(a, b), step: Number.isFinite(step) && step > 0 ? step : null };
}

/** The effect's `clamp` flag (section 11): values above 1.0 pass through unless it is set. */
const clampFlag = (def: object): boolean => (def as { clamp?: unknown }).clamp === true;

export function effectSnapshot(def: EffectDefinition, origin: DefinitionOrigin, preambles: Preambles): DefinitionSnapshot {
  const legacy = toLegacyFx(def, preambles.effect);
  const glsl = def.rawShader ? wrappedGlsl(legacy.frag, legacy.frag) : wrappedGlsl(legacy.frag, def.frag, '\n}\n');
  const params = def.params.map(paramSpec);
  const numeric = new Set(params.filter((p) => p.type === 'number').map((p) => p.key));
  return {
    id: def.id, kind: 'effect', version: 0, origin, renderer: null, glsl, params,
    flags: {
      passes: legacy.passes, keepOrig: legacy.keepOrig, backdrop: legacy.backdrop, rawShader: def.rawShader === true,
      viewportSafe: def.viewportSafe === true,
      viewportPadding: (def.viewportPadding ?? []).filter((key) => numeric.has(key)),
      clamp: clampFlag(def)
    }
  };
}

export function transitionSnapshot(def: TransitionDefinition, origin: DefinitionOrigin, preambles: Preambles): DefinitionSnapshot {
  const legacy = toLegacyTransition(def, preambles.transition);
  const glsl = def.rawShader ? wrappedGlsl(legacy.frag, legacy.frag) : wrappedGlsl(legacy.frag, def.frag, '\n}\n');
  return {
    id: def.id, kind: 'transition', version: 0, origin, renderer: null, glsl, params: def.params.map(paramSpec),
    flags: { passes: 1, keepOrig: false, backdrop: false, rawShader: def.rawShader === true, viewportSafe: false, viewportPadding: [], clamp: clampFlag(def) }
  };
}

export function layerSnapshot(def: ExtensionLayerDefinition, origin: DefinitionOrigin, preambles: Preambles): DefinitionSnapshot {
  const r = def.renderer;
  const renderer: LayerRenderer = r.kind === 'fragment' ? { kind: 'fragment', fragment: r.fragment }
    : r.kind === 'mesh' ? { kind: 'mesh', assetField: r.assetField }
    : r.kind === 'scene3d' ? { kind: 'scene3d' }
    : { kind: 'layer3d', role: r.role };
  const glsl = r.kind === 'fragment' ? wrappedGlsl(extensionLayerFragment(def, preambles.layer), r.fragment) : null;
  return {
    id: def.id, kind: 'layer', version: Math.max(0, Math.floor(finite(def.version, 0))), origin, renderer, glsl, params: def.params.map(paramSpec),
    flags: { passes: 1, keepOrig: false, backdrop: false, rawShader: false, viewportSafe: false, viewportPadding: [], clamp: false }
  };
}

/* ── the publisher ─────────────────────────────────────────────────── */

export interface DefinitionRegistries {
  effects: Registry<EffectDefinition>;
  transitions: Registry<TransitionDefinition>;
  layerTypes: Registry<ExtensionLayerDefinition>;
}

/** A change to the winning set (no generation: each sink numbers its own). */
export interface DefinitionChange {
  upserts: DefinitionSnapshot[];
  removals: DefinitionRemoval[];
}

export interface PublisherOptions {
  /** Where an owner's registrations come from. */
  origin(ownerId: string): DefinitionOrigin;
  preambles(): Preambles;
  /** A definition the kernel accepted but cannot be snapshotted (logged; it is left out). */
  onError?(kind: DefinitionKind, id: string, error: unknown): void;
}

const keyOf = (kind: DefinitionKind, id: string) => `${kind}\0${id}`;

export class DefinitionPublisher {
  private listeners = new Set<(change: DefinitionChange) => void>();
  private pending = new Set<string>();
  private scheduled = false;
  private offs: Array<{ dispose(): void }> = [];

  constructor(private readonly registries: DefinitionRegistries, private readonly options: PublisherOptions) {
    const watch = (kind: DefinitionKind, registry: Registry<{ id: string }>) =>
      this.offs.push(registry.onChange((change) => this.changed(kind, change.id)));
    watch('effect', registries.effects as unknown as Registry<{ id: string }>);
    watch('transition', registries.transitions as unknown as Registry<{ id: string }>);
    watch('layer', registries.layerTypes as unknown as Registry<{ id: string }>);
  }

  /** The winner for one id, or null when nothing is registered (or it cannot be snapshotted). */
  snapshot(kind: DefinitionKind, id: string): DefinitionSnapshot | null {
    const registry = (kind === 'effect' ? this.registries.effects : kind === 'transition' ? this.registries.transitions : this.registries.layerTypes) as unknown as Registry<{ id: string }>;
    const entry = registry.topEntry(id);
    if (!entry) return null;
    try {
      const origin = this.options.origin(entry.ownerId);
      const preambles = this.options.preambles();
      if (kind === 'effect') return effectSnapshot(entry.item as EffectDefinition, origin, preambles);
      if (kind === 'transition') return transitionSnapshot(entry.item as TransitionDefinition, origin, preambles);
      return layerSnapshot(entry.item as ExtensionLayerDefinition, origin, preambles);
    } catch (error) {
      this.options.onError?.(kind, id, error);
      return null;
    }
  }

  /** Every winning definition. */
  all(): DefinitionSnapshot[] {
    const out: DefinitionSnapshot[] = [];
    const add = (kind: DefinitionKind, ids: string[]) => {
      for (const id of ids) {
        const s = this.snapshot(kind, id);
        if (s) out.push(s);
      }
    };
    add('effect', this.registries.effects.ids());
    add('transition', this.registries.transitions.ids());
    add('layer', this.registries.layerTypes.ids());
    return out;
  }

  subscribe(fn: (change: DefinitionChange) => void): () => void {
    this.listeners.add(fn);
    return () => void this.listeners.delete(fn);
  }

  /** Delivers pending changes now (tests; the microtask does it otherwise). */
  flush(): void {
    this.scheduled = false;
    if (!this.pending.size) return;
    const keys = [...this.pending];
    this.pending.clear();
    const change: DefinitionChange = { upserts: [], removals: [] };
    for (const key of keys) {
      const [kind, id] = key.split('\0') as [DefinitionKind, string];
      const snapshot = this.snapshot(kind, id);
      if (snapshot) change.upserts.push(snapshot);
      else change.removals.push({ id, kind });
    }
    for (const fn of [...this.listeners]) {
      try { fn(change); } catch (error) { console.error('[definitions] listener failed', error); }
    }
  }

  dispose(): void {
    for (const off of this.offs) off.dispose();
    this.offs = [];
    this.listeners.clear();
  }

  private changed(kind: DefinitionKind, id: string): void {
    this.pending.add(keyOf(kind, id));
    if (this.scheduled) return;
    this.scheduled = true;
    queueMicrotask(() => this.flush());
  }
}

/**
 * One receiver's stream: a full update, then deltas numbered by one. `send`
 * returns false (or throws) when the receiver refused an update, which makes
 * the next one full again (a generation gap or an engine restart).
 */
export class SessionDefinitions {
  private generation = 0;
  private needsFull = true;
  private off: (() => void) | null;
  /** The generation of the latest update that upserted or removed each definition (`kind\0id`). */
  private changed = new Map<string, number>();
  /** The generation of the latest full update (it changes every definition). */
  private fullAt = 0;

  constructor(private readonly publisher: DefinitionPublisher, private readonly send: (update: DefinitionsUpdate) => boolean | void, private onDispose: (() => void) | null = null) {
    this.off = publisher.subscribe((change) => this.delta(change));
    this.full();
  }

  /** Sends the whole set (session open, after a gap, after an engine restart). */
  full(): void {
    this.needsFull = false;
    this.deliver({ generation: ++this.generation, full: true, upserts: this.publisher.all(), removals: [] });
  }

  /** The generation of the latest update sent that changed this definition. */
  changedAt(kind: string, id: string): number | undefined {
    const at = Math.max(this.changed.get(keyOf(kind as DefinitionKind, id)) ?? 0, this.fullAt);
    return at || undefined;
  }

  /** The generation of the latest update sent. */
  get latest(): number {
    return this.generation;
  }

  /** The receiver lost track (refusals it stopped retrying): the next change sends the whole set. */
  stale(): void {
    this.needsFull = true;
  }

  dispose(): void {
    this.off?.();
    this.off = null;
    this.onDispose?.();
    this.onDispose = null;
  }

  private delta(change: DefinitionChange): void {
    if (this.needsFull) return this.full();
    if (!change.upserts.length && !change.removals.length) return;
    this.deliver({ generation: ++this.generation, full: false, upserts: change.upserts, removals: change.removals });
  }

  private deliver(update: DefinitionsUpdate): void {
    if (update.full) this.fullAt = update.generation;
    for (const d of [...update.upserts, ...update.removals]) this.changed.set(keyOf(d.kind, d.id), update.generation);
    let ok: boolean | void = true;
    try { ok = this.send(update); } catch { ok = false; }
    if (ok === false) this.needsFull = true;
  }
}

/* ── origin from the extension records ─────────────────────────────── */

export interface OwnerRecord {
  scope?: string;
  trust?: string;
  manifest?: { forkedFrom?: string; replaces?: string[] } | null;
}

/**
 * Built-in for built-in extensions; fork for a local extension that forks or
 * replaces a built-in (it registers under the built-in ids, section 11);
 * Store for a sandboxed Store extension; local otherwise (in-realm,
 * agent-published, trusted Store installs, unknown owners).
 */
export function originOf(record: OwnerRecord | undefined | null): DefinitionOrigin {
  if (!record) return 'local';
  if (record.scope === 'builtin' || record.trust === 'builtin') return 'builtIn';
  if (record.manifest?.forkedFrom || record.manifest?.replaces?.length) return 'fork';
  if (record.trust === 'store') return 'store';
  return 'local';
}
