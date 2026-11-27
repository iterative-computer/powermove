/*
 * Which corpus projects today's engine can give a reference for.
 *
 * Today's engine ignores a layer's `compat`: it draws every text layer at
 * compatibility level 1. A capture of a level 2 text document is therefore
 * a level 1 picture of it, not a level 2 reference, and a parity gate that
 * compared the native level 2 result with it would fail by design (level 2
 * lays text out with the engine's improved semantics, plan Appendix A).
 * Those documents are judged by the native semantic gate
 * (`engine/crates/pm-render/tests/text_level2_semantics.rs`) and stay in
 * `allProjects()` for generation, validation and routing checks; the legacy
 * capture leaves them out and records that it did. The Rust gates route any
 * level 2 document an older capture still holds to the same semantic gate.
 */
import type { CorpusProject } from './projects/common';

type Layer = Record<string, any>;

function layerLists(doc: Record<string, any>): Layer[][] {
  return [doc.layers ?? [], ...Object.values(doc.comps ?? {}).map((c: any) => c.layers ?? [])];
}

/** Whether a document holds a text layer at compatibility level 2 or above. */
export function hasLevel2Text(doc: Record<string, any>): boolean {
  return layerLists(doc).some(list => list.some(l => l?.type === 'text' && Number(l.compat) >= 2));
}

/** The projects a legacy capture records, and the native-only ones it leaves
 * to the semantic gate, each in the given order. */
export function legacyCaptureProjects(projects: CorpusProject[]): { capture: CorpusProject[]; nativeOnly: CorpusProject[] } {
  const capture: CorpusProject[] = [];
  const nativeOnly: CorpusProject[] = [];
  for (const item of projects) (hasLevel2Text(item.doc) ? nativeOnly : capture).push(item);
  return { capture, nativeOnly };
}
