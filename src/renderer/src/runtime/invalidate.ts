/*
 * The invalidation edge — replaces PM.touch() + PM.invalidate().
 *
 * Synchronous by contract: memo caches must die in the same call that reports
 * a mutation (the world-matrix memo is gated on memoT === T and is NOT cleared
 * per frame, so deferring this would leave a paused frame reading stale
 * matrices — see the legacy editing.js cancel() comment).
 *
 * Two consumers, two mechanisms:
 *   - the rAF loop polls plain `frame` flags (zero reactive cost);
 *   - the Svelte layer gets tick bumps (batched per edit transaction by the
 *     caller — invalidate() itself is cheap enough to call per command).
 */
import { doc } from '../state/document.svelte';
import type { FrameFlags, MutationKind } from './ports';

export const frame: FrameFlags = { render: true, timeline: true, status: false };

type CacheClearer = () => void;
const cacheClearers = new Set<CacheClearer>();

/** Register a synchronous cache reset (anim memos, raster caches, …). */
export function onInvalidate(clearer: CacheClearer): () => void {
  cacheClearers.add(clearer);
  return () => {
    cacheClearers.delete(clearer);
  };
}

let mutationSeq = 0;
/** Monotonic mutation counter — the snapshot memo key. */
export const mutationSequence = (): number => mutationSeq;

export function invalidate(kind: MutationKind = 'values'): void {
  mutationSeq++;
  for (const clear of [...cacheClearers]) clear();
  frame.render = true;
  frame.timeline = true;
  doc.bumpFor(kind);
}

/** View-only invalidation (zoom, quality): repaint without a document tick. */
export function invalidateView(what: 'render' | 'timeline' | 'status' | 'all' = 'all'): void {
  if (what === 'all' || what === 'render') frame.render = true;
  if (what === 'all' || what === 'timeline') frame.timeline = true;
  if (what === 'status') frame.status = true;
}
