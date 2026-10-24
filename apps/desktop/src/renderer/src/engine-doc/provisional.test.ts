/*
 * Provisional revisions (barrier.ts `flushProvisional`): an unscoped write's
 * recently changed layers leave first, the rest of the document is compared
 * after, and the comparison confirms the revision or supersedes it. Every
 * stream item is checked against the live document: a confirmed revision's
 * mirror is the whole document, a superseded one is never confirmed.
 */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { canonicalProjectHash } from '../../../shared/canonical-json';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc, type EngineDoc } from './install';
import { parseMode } from './flags';
import type { PatchBatch } from './barrier';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));

async function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode('on'), loadModule, sink: () => {} });
  PM.proj = PM.mkProject({ dur: 10 });
  PM.proj.layers = Array.from({ length: 6 }, (_, i) => Object.assign(PM.mkLayer('solid', { name: `L${i}` }), { id: `L${i}` }));
  expect(await doc.whenReady).toBe(true);
  const barrier = doc.barrier!;
  barrier.flush();
  const items: PatchBatch[] = [];
  barrier.subscribe(batch => items.push(batch));
  const layer = (id: string) => PM.proj.layers.find((l: any) => l.id === id);
  /** One unscoped write, as an extension's `touch()` makes it. */
  const write = (fn: () => void) => { fn(); PM.touch(); };
  return { PM, doc, barrier, items, layer, write };
}

describe.skipIf(!existsSync(MODULE))('provisional revisions', () => {
  it('sends the layer an unscoped write changed last first, then confirms it when nothing else changed', async () => {
    const { PM, doc, barrier, items, layer, write } = await editor();
    write(() => { layer('L2').p['position.x'].v = 10; });
    expect(barrier.flushProvisional()).toBe(false); // Nothing is known to be hot yet.
    barrier.flush();
    items.length = 0;
    write(() => { layer('L2').p['position.x'].v = 20; });
    expect(barrier.flushProvisional()).toBe(true);
    const provisional = items.at(-1)!;
    expect(provisional.provisional).toBe(true);
    expect(barrier.provisionalRevision).toBe(provisional.revision);
    barrier.flush();
    expect(items.at(-1)).toMatchObject({ confirmed: true, revision: provisional.revision, count: 0 });
    expect(barrier.provisionalRevision).toBe(null);
    expect(doc.wasm!.hash()).toBe(canonicalProjectHash(PM.proj));
  });

  it('supersedes the provisional revision when the write also changed a layer it did not change before', async () => {
    const { PM, doc, barrier, items, layer, write } = await editor();
    write(() => { layer('L2').p['position.x'].v = 10; });
    barrier.flush();
    items.length = 0;
    // The adversarial write: the hot layer and another one, in one touch().
    write(() => { layer('L2').p['position.x'].v = 30; layer('L5').p.opacity.v = 40; });
    expect(barrier.flushProvisional()).toBe(true);
    const provisional = items.at(-1)!;
    barrier.flush();
    const rest = items.at(-1)!;
    expect(rest.confirmed).toBeUndefined();
    expect(rest.provisional).toBeUndefined();
    expect(rest.revision).toBe(provisional.revision + 1);
    expect(items.some(item => item.confirmed)).toBe(false);
    expect(barrier.provisionalRevision).toBe(null);
    expect(doc.wasm!.hash()).toBe(canonicalProjectHash(PM.proj));
  });

  it('never confirms a revision the live document has moved past, with edits back to back', async () => {
    const { PM, doc, barrier, items, layer, write } = await editor();
    write(() => { layer('L1').p['position.x'].v = 1; });
    barrier.flush();
    items.length = 0;
    for (let i = 0; i < 40; i++) {
      write(() => {
        layer('L1').p['position.x'].v = 100 + i;
        if (i % 3 === 0) layer(`L${i % 6}`).p['position.y'].v = i;
      });
      barrier.flushProvisional();
      // A second write before the comparison: it must be in what is confirmed.
      if (i % 4 === 1) write(() => { layer('L4').p.rotation.v = i; });
      // A query mid-way flushes everything (only the edit's own answers
      // read the provisional mirror).
      if (i % 5 === 2) barrier.provisionalQueries(() => barrier.flush());
      barrier.flush();
      expect(barrier.provisionalRevision).toBe(null);
      expect(doc.wasm!.hash()).toBe(canonicalProjectHash(PM.proj));
    }
    // Every confirmation names the newest revision sent, which was provisional.
    for (const [i, item] of items.entries()) {
      if (!item.confirmed) continue;
      const sent = items.slice(0, i).filter(b => !b.confirmed).at(-1)!;
      expect(sent.provisional).toBe(true);
      expect(item.revision).toBe(sent.revision);
    }
    expect(items.some(item => item.confirmed)).toBe(true);
    expect(items.some(item => item.provisional) && items.some(item => !item.provisional && !item.confirmed)).toBe(true);
  });

  it('answers queries from the provisional mirror only inside provisionalQueries', async () => {
    const { PM, barrier, layer, write } = await editor();
    write(() => { layer('L3').p['position.x'].v = 5; });
    barrier.flush();
    write(() => { layer('L3').p['position.x'].v = 6; layer('L0').p['position.x'].v = 7; });
    expect(barrier.flushProvisional()).toBe(true);
    const provisional = barrier.revision;
    expect(barrier.provisionalQueries(() => { barrier.flush(); return barrier.revision; })).toBe(provisional);
    barrier.flush();
    expect(barrier.revision).toBe(provisional + 1);
  });
});
