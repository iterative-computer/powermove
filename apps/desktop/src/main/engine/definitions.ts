/*
 * The definition-snapshot relay (contracts 10 and 14).
 *
 * Each window's kernel pushes its definition updates (register, dispose,
 * hot reload) to its own engine sessions. Main keeps the latest set per
 * window with the same rules the engine applies (pm_protocol
 * `DefinitionSet::apply`), so sessions main opens itself (export and
 * agent job snapshots in the job engine) start from the window's current
 * definitions, and so a restarted engine can be primed.
 */
import type { DefinitionKind, DefinitionSnapshot, DefinitionsUpdate } from '../../shared/engine-protocol';
import { isRecord, isSafeUint } from './protocol';

export class DefinitionGap extends Error {
  override name = 'DefinitionGap';
}

const key = (kind: DefinitionKind, id: string): string => `${kind}\0${id}`;

/** pm_protocol `DefinitionSet`. */
export class DefinitionSet {
  private gen: number | null = null;
  private definitions = new Map<string, DefinitionSnapshot>();
  private missing = new Map<string, string>();

  get generation(): number | null {
    return this.gen;
  }

  apply(update: DefinitionsUpdate): void {
    validateUpdate(update);
    const follows = this.gen === null ? update.full : update.full ? update.generation > this.gen : update.generation === this.gen + 1;
    if (!follows) throw new DefinitionGap(`update generation ${update.generation} does not follow ${this.gen}; send a full update`);
    if (update.full) {
      for (const [k, snapshot] of this.definitions) {
        if (!update.upserts.some((d) => key(d.kind, d.id) === k)) this.missing.set(k, snapshot.id);
      }
      this.definitions.clear();
    }
    for (const snapshot of update.upserts) {
      const k = key(snapshot.kind, snapshot.id);
      this.missing.delete(k);
      this.definitions.set(k, snapshot);
    }
    for (const removal of update.removals) {
      const k = key(removal.kind, removal.id);
      this.definitions.delete(k);
      this.missing.set(k, removal.id);
    }
    this.gen = update.generation;
  }

  /** A full update carrying the whole set at `generation`. */
  fullUpdate(generation: number): DefinitionsUpdate {
    return { generation, full: true, upserts: [...this.definitions.values()], removals: [] };
  }

  get size(): number {
    return this.definitions.size;
  }

  missingIds(): string[] {
    return [...this.missing.values()];
  }
}

/** Shape checks for an update a renderer sent (the engine validates the
 *  definitions themselves when it receives them). */
export function validateUpdate(update: unknown): asserts update is DefinitionsUpdate {
  if (!isRecord(update)) throw new DefinitionGap('a definitions update is an object');
  if (!isSafeUint(update['generation']) || typeof update['full'] !== 'boolean' || !Array.isArray(update['upserts']) || !Array.isArray(update['removals'])) {
    throw new DefinitionGap('a definitions update has a generation, full, upserts, and removals');
  }
  if (update['full'] && update['removals'].length) throw new DefinitionGap('a full update has no removals');
  const seen = new Set<string>();
  for (const item of [...update['upserts'], ...update['removals']] as unknown[]) {
    if (!isRecord(item) || typeof item['id'] !== 'string' || !item['id'] || (item['kind'] !== 'effect' && item['kind'] !== 'transition' && item['kind'] !== 'layer')) {
      throw new DefinitionGap('each definition has an id and a kind');
    }
    const k = key(item['kind'], item['id']);
    if (seen.has(k)) throw new DefinitionGap(`definition ${item['id']} appears twice in one update`);
    seen.add(k);
  }
}

/** Latest definitions per window. */
export class DefinitionRelay {
  private windows = new Map<number, DefinitionSet>();

  update(windowId: number, update: unknown): void {
    validateUpdate(update);
    const set = this.windows.get(windowId);
    if (!set) {
      if (!update.full) throw new DefinitionGap('the first update from a window must be full');
      const fresh = new DefinitionSet();
      fresh.apply(update);
      this.windows.set(windowId, fresh);
      return;
    }
    // A reloaded window starts its generations again with a full update.
    if (update.full && set.generation !== null && update.generation <= set.generation) {
      const fresh = new DefinitionSet();
      fresh.apply(update);
      this.windows.set(windowId, fresh);
      return;
    }
    set.apply(update);
  }

  /** A full update for a session main opens for this window. */
  snapshotFor(windowId: number, generation = 1): DefinitionsUpdate | null {
    return this.windows.get(windowId)?.fullUpdate(generation) ?? null;
  }

  forget(windowId: number): void {
    this.windows.delete(windowId);
  }
}
