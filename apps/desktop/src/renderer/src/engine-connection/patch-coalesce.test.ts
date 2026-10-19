/*
 * One patch batch per synchronous run (patch-coalesce.ts): a run's batches of
 * one session and origin reach the engine as one batch whose operations are
 * the batches' in order; anything else sent meanwhile keeps the page's order.
 */
import { describe, expect, it } from 'vitest';

import type { Envelope } from '../../../shared/engine-protocol';
import { decodePatchOps, encodePatchOps } from '../../../shared/patch-binary';
import { decodeEnvelope } from '../../../main/engine/protocol';
import { applyOps, type PatchOp } from '../../../shared/patch-ids';
import { EngineConnection, type PortLike } from './connection';
import { canMerge, coalesce, mergeBatches, type ApplyPatches } from './patch-coalesce';

class Port implements PortLike {
  sent: Envelope[] = [];
  postMessage(message: unknown): void {
    this.sent.push(decodeEnvelope(message as Uint8Array));
  }
  addEventListener(): void {}
  start(): void {}
  close(): void {}
  types(): string[] {
    return this.sent.map((e) => (e.message.kind === 'event' ? (e.message.body as { type: string }).type : e.message.kind));
  }
}

const x: PatchOp[] = [{ path: ['comps', 'C', 'layers', 3, 'p', 'position.x', 'v'], exists: true, value: 120.5 }];
const y: PatchOp[] = [
  { path: ['comps', 'C', 'layers', 3, 'p', 'position.y', 'v'], exists: true, value: -4 },
  { path: ['comps', 'C', 'layers', 3, 'name'], exists: true, value: 'Solid ✓' }
];

function batch(base: number, revision: number, ops: PatchOp[], origin: ApplyPatches['origin'] = 'edit'): ApplyPatches {
  return { type: 'applyPatches', document: 'P', baseRevision: base, revision, origin, encoding: 'canonicalV1', patchCount: ops.length, body: encodePatchOps(ops) };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('one patch batch per run', () => {
  it('merges contiguous batches into one whose operations apply as the batches do', () => {
    const merged = mergeBatches(batch(3, 4, x), batch(4, 5, y));
    expect([merged.baseRevision, merged.revision, merged.patchCount]).toEqual([3, 5, 3]);
    expect(decodePatchOps(merged.body)).toEqual([...decodePatchOps(encodePatchOps(x)), ...decodePatchOps(encodePatchOps(y))]);
    // The same bytes as encoding the operations together.
    expect(Array.from(merged.body)).toEqual(Array.from(encodePatchOps([...x, ...y])));
    const doc = { comps: { C: { layers: [{}, {}, {}, { name: 'a', p: { 'position.x': { v: 0 }, 'position.y': { v: 0 } } }] } } };
    const one = structuredClone(doc);
    applyOps(one, decodePatchOps(merged.body));
    const two = structuredClone(doc);
    applyOps(two, x);
    applyOps(two, y);
    expect(one).toEqual(two);
  });

  it('keeps apart what does not continue: another origin, document, gap, or size', () => {
    expect(canMerge(batch(3, 4, x), batch(4, 5, y), 1 << 20)).toBe(true);
    expect(canMerge(batch(3, 4, x), batch(4, 5, y, 'undo'), 1 << 20)).toBe(false);
    expect(canMerge(batch(3, 4, x), { ...batch(4, 5, y), document: 'Q' }, 1 << 20)).toBe(false);
    expect(canMerge(batch(3, 4, x), batch(5, 6, y), 1 << 20)).toBe(false);
    expect(canMerge(batch(3, 4, x), batch(4, 5, y), 16)).toBe(false);
    expect(canMerge(batch(3, 4, x), { ...batch(4, 5, y), body: new Uint8Array([1, 2, 3]) }, 1 << 20)).toBe(false);
    // Sessions interleaved keep their order; each session's runs merge.
    const out = coalesce(
      [
        { body: batch(3, 4, x), session: 1 },
        { body: batch(4, 5, y), session: 1 },
        { body: batch(9, 10, x), session: 2 },
        { body: batch(5, 6, x), session: 1 }
      ],
      1 << 20
    );
    expect(out.map((o) => [o.session, o.body.baseRevision, o.body.revision])).toEqual([
      [1, 3, 5],
      [2, 9, 10],
      [1, 5, 6]
    ]);
  });

  it('sends a run\'s batches once, at its end, and before anything sent after them', async () => {
    const port = new Port();
    const connection = new EngineConnection(1, port, () => {});
    // A drag move: two flushes in one handler.
    connection.patches(batch(3, 4, x), 5, 1 << 20);
    connection.patches(batch(4, 5, y), 5, 1 << 20);
    expect(port.types()).toEqual([]);
    await tick();
    expect(port.types()).toEqual(['applyPatches']);
    const body = port.sent[0]!.message as { body: ApplyPatches };
    expect([body.body.baseRevision, body.body.revision]).toEqual([3, 5]);
    // A viewer state sent between two batches goes after the first.
    connection.patches(batch(5, 6, x), 5, 1 << 20);
    connection.event({ type: 'viewer', composition: 'C' } as never, 5);
    connection.patches(batch(6, 7, y), 5, 1 << 20);
    await tick();
    expect(port.types()).toEqual(['applyPatches', 'applyPatches', 'viewer', 'applyPatches']);
    const revisions = port.sent
      .map((e) => (e.message.kind === 'event' ? (e.message.body as ApplyPatches) : null))
      .filter((b): b is ApplyPatches => b?.type === 'applyPatches')
      .map((b) => [b.baseRevision, b.revision]);
    expect(revisions).toEqual([[3, 5], [5, 6], [6, 7]]);
    // A closed connection drops what it held.
    connection.patches(batch(7, 8, x), 5, 1 << 20);
    connection.close('test');
    await tick();
    expect(port.types()).toHaveLength(4);
  });
});
