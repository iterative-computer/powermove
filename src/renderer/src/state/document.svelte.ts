/*
 * Document store — the hybrid at the heart of the renderer:
 *
 *   proj  : $state.raw — a plain object graph. Reading `doc.proj` in a derived
 *           tracks only REFERENCE SWAPS (undo, load, rollback). PM.Edit mutates
 *           it in place; the compositor reads it via ports.getProject() with
 *           zero signal reads. NEVER wrap layers in a deep $state proxy.
 *   tick  : version counters. Every derived over project data reads the tick(s)
 *           it depends on plus `doc.proj`; invalidate() bumps them once per
 *           edit transaction, so the derived graph replaces the legacy
 *           `I.syncs[]` closure list and the bus listeners.
 *   generation : identity counter for consumers that used to compare
 *           `state.project !== PM.proj` (audio reconcile).
 */
import type { Project } from '../core/types/project';
import type { MutationKind } from '../runtime/ports';

export type TickScope = Exclude<MutationKind, 'replace'>;

const TICK_SCOPES: readonly TickScope[] = ['values', 'structure', 'project', 'assets', 'library', 'history'];

class DocumentStore {
  proj: Project = $state.raw(emptyProject());
  tick: Record<TickScope, number> = $state({ values: 0, structure: 0, project: 0, assets: 0, library: 0, history: 0 });
  generation = $state(0);

  bump(...scopes: TickScope[]): void {
    for (const scope of scopes) this.tick[scope]++;
  }

  bumpFor(kind: MutationKind): void {
    switch (kind) {
      case 'values': this.bump('values'); break;
      case 'structure': this.bump('structure', 'values'); break;
      case 'project': this.bump('project', 'values'); break;
      case 'assets': this.bump('assets'); break;
      case 'library': this.bump('library'); break;
      case 'history': this.bump('history'); break;
      case 'replace': this.bump(...TICK_SCOPES); break;
    }
  }

  /** The one restore path. Swapping the raw reference re-runs every derived. */
  replace(next: Project): void {
    this.proj = next;
    this.generation++;
    this.bumpFor('replace');
  }
}

export const doc = new DocumentStore();

function emptyProject(): Project {
  // Populated by the composition root before anything renders.
  return {} as Project;
}
