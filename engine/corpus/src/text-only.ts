/*
 * Text in isolation (`capture.ts --text-only`): each corpus project that has
 * text or captions layers, reduced to those layers, so the text closure
 * (`engine/crates/pm-render/tests/text_corpus.rs`) compares every text layer
 * of the corpus even where the rest of its project draws with features the
 * native engine does not draw yet (SVG images, motion blur, 2.5D, path
 * masks). Text and captions layers keep their own properties, animators,
 * masks and built-in effects; groups, precompositions and nulls stay for
 * their transforms. Everything else is removed, and the variant is flat and sharp
 * (no 3D switch, no motion blur) and without extension effects, which the
 * engine's comparison has no definition for.
 */
import { EXTENSION_EFFECTS } from './catalog';
import type { CorpusProject } from './projects/common';

const KEPT = new Set(['text', 'captions', 'group', 'precomp', 'null']);

type Layer = Record<string, any>;

function layerLists(doc: Record<string, any>): Layer[][] {
  return [doc.layers ?? [], ...Object.values(doc.comps ?? {}).map((c: any) => c.layers ?? [])];
}

/** Whether a project document has a text or captions layer. */
export function hasText(doc: Record<string, any>): boolean {
  return layerLists(doc).some(list => list.some(l => l.type === 'text' || l.type === 'captions'));
}

/** The document reduced to its text (see the module comment). */
export function textOnly(doc: Record<string, any>): Record<string, any> {
  const copy = structuredClone(doc);
  const reduce = (layers: Layer[]): Layer[] => {
    const kept = layers.filter(l => KEPT.has(l.type));
    const ids = new Set(kept.map(l => l.id));
    return kept.map(l => {
      const out: Layer = { ...l, threeD: false, mblur: false };
      // A parent or matte that is not kept goes with its layer.
      if (out.parent && !ids.has(out.parent)) delete out.parent;
      if (out.matteSource && !ids.has(out.matteSource)) out.matteSource = null;
      if (Array.isArray(out.fx)) out.fx = out.fx.filter((f: any) => !(f?.type in EXTENSION_EFFECTS));
      return out;
    });
  };
  copy.layers = reduce(copy.layers ?? []);
  for (const comp of Object.values(copy.comps ?? {}) as any[]) comp.layers = reduce(comp.layers ?? []);
  return copy;
}

/** The corpus projects with text, reduced to it; no extensions to install. */
export function textOnlyProjects(projects: CorpusProject[]): CorpusProject[] {
  return projects
    .filter(p => hasText(p.doc))
    .map(p => ({ ...p, doc: textOnly(p.doc), extensions: [], covers: [...(p.covers ?? []), 'text in isolation'] }));
}
