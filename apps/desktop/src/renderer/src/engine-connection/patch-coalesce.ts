/*
 * One patch batch per synchronous run (docs/engine/perf.md, "One batch per
 * move").
 *
 * The write barrier flushes before every pm-wasm query, so one handler can
 * produce several batches: a layer drag's move sets `position.x`, then
 * `position.y`, and queries in between, which made two batches about half a
 * millisecond apart. Sent as they came, the engine drew a frame for the
 * first (the layer moved only horizontally) and the second waited for that
 * render: two renders per move, and the move's own frame about 2 ms late.
 *
 * The connection holds a run's batches until the run ends (a microtask) and
 * sends each session's contiguous batches of one origin as one: the first
 * batch's base revision, the last one's revision, and the operations of each
 * in order. Applying the merged operations is applying the batches one after
 * another (the format is a count and a list of operations, contract 5).
 * Anything else the connection sends goes after the batches held before it,
 * so the engine sees every message in the order the page sent it.
 */
import type { Event } from '../../../shared/engine-protocol';
import { PATCH_BINARY_MAGIC } from '../../../shared/patch-binary';

export type ApplyPatches = Extract<Event, { type: 'applyPatches' }>;

/** Magic and operation count. */
const HEADER = PATCH_BINARY_MAGIC.length + 4;

function operations(body: Uint8Array): number | null {
  if (body.length < HEADER || PATCH_BINARY_MAGIC.some((byte, i) => body[i] !== byte)) return null;
  return new DataView(body.buffer, body.byteOffset, body.byteLength).getUint32(PATCH_BINARY_MAGIC.length, true);
}

/** Whether `next` continues `prev` and the merged body stays within `limit` bytes. */
export function canMerge(prev: ApplyPatches, next: ApplyPatches, limit: number): boolean {
  return (
    prev.document === next.document &&
    prev.origin === next.origin &&
    prev.encoding === 'canonicalV1' &&
    next.encoding === 'canonicalV1' &&
    next.baseRevision === prev.revision &&
    operations(prev.body) !== null &&
    operations(next.body) !== null &&
    prev.body.length + next.body.length - HEADER <= limit
  );
}

/** `prev` then `next` as one batch (`canMerge` must hold). */
export function mergeBatches(prev: ApplyPatches, next: ApplyPatches): ApplyPatches {
  const count = operations(prev.body)! + operations(next.body)!;
  const body = new Uint8Array(prev.body.length + next.body.length - HEADER);
  body.set(prev.body.subarray(0, PATCH_BINARY_MAGIC.length), 0);
  new DataView(body.buffer).setUint32(PATCH_BINARY_MAGIC.length, count, true);
  body.set(prev.body.subarray(HEADER), HEADER);
  body.set(next.body.subarray(HEADER), prev.body.length);
  return { ...prev, revision: next.revision, patchCount: prev.patchCount + next.patchCount, body };
}

/** Held batches in the order they were sent, each session's contiguous runs merged. */
export function coalesce(pending: ReadonlyArray<{ body: ApplyPatches; session: number }>, limit: number): Array<{ body: ApplyPatches; session: number }> {
  const out: Array<{ body: ApplyPatches; session: number }> = [];
  for (const item of pending) {
    const last = out.at(-1);
    if (last && last.session === item.session && canMerge(last.body, item.body, limit)) {
      last.body = mergeBatches(last.body, item.body);
    } else {
      out.push({ ...item });
    }
  }
  return out;
}
