import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { canonicalProject } from '../../../shared/canonical-json';
import { decodePatchOps } from '../../../shared/patch-binary';
import { applyOps, type PatchOp } from '../../../shared/patch-ids';
import { makePM } from '../legacy/__tests__/make-pm';
import { ConsistencyHash } from './consistency-hash';
import { anchorPath, DocumentBarrier } from './barrier';
import { installEngineDoc } from './install';
import { parseMode } from './flags';
import { diffJsonOps } from './json-diff';
import { notifyDocument } from './write';
import { imports, PmWasm } from './wasm';

const module = WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));
const wasm = async () => new PmWasm(await WebAssembly.instantiate(await module, imports));
const large = (layers = 96, keys = 1000) => ({ id: 'synthetic-large', compId: 'comp', compName: 'Large', comps: {}, assets: {}, layers: Array.from({ length: layers }, (_, i) => ({ id: `layer-${i}`, p: { x: { v: i, kf: Array.from({ length: keys }, (_, k) => ({ i: `key-${i}-${k}`, t: k / 30, v: i + k, ease: { x: 0.5, y: 0.5 } })) } } })) });

describe('incremental large-document synchronization', () => {
  it('hashes independently in JS and wasm and reuses unchanged subtrees after structural operations', async () => {
    const instance = await wasm();
    const project = large(24, 100);
    const barrier = new DocumentBarrier(instance, () => project);
    barrier.resync();
    expect(barrier.check('initial').ok).toBe(true);
    const before = instance.consistencyHash();
    project.layers.reverse();
    project.layers[0]!.p.x.kf[50]!.v = 900;
    barrier.touched([['layers']]);
    barrier.flush();
    const hash = new ConsistencyHash();
    expect(instance.consistencyHash()).toBe(hash.digest(barrier.shadow));
    expect(instance.consistencyHash()).not.toBe(before);
    expect(barrier.check('reorder and edit').ok).toBe(true);
    // Raw retained references are part of the compatibility contract: a
    // consistency check still finds a write without touch or a bus event.
    project.layers[1]!.p.x.kf[40]!.v = 901;
    expect(barrier.check('missed alias').ok).toBe(false);
    expect(barrier.check('repaired').ok).toBe(true);
    Object.defineProperty(project.layers[0]!.p.x, 'toJSON', { configurable: true, value: () => ({ v: -7, kf: [] }) });
    expect(barrier.check('custom toJSON').ok).toBe(false);
    expect(barrier.check('custom toJSON repaired').ok).toBe(true);
  });

  it('keeps a self-returning toJSON and nested hooks consistent with the real wasm mirror', async () => {
    const instance = await wasm();
    const project = large(2, 4);
    const channel: any = project.layers[0]!.p.x;
    channel.nested = { toJSON: () => ({ value: 7 }) };
    Object.defineProperty(channel, 'toJSON', { value() { return this; } });
    const barrier = new DocumentBarrier(instance, () => project);
    barrier.resync();
    expect(barrier.check('self-returning hook').ok).toBe(true);
    channel.v = 81; // Raw alias: no touch or announcement.
    expect(barrier.check('missed raw alias').ok).toBe(false);
    expect(barrier.check('repaired raw alias').ok).toBe(true);
    expect(instance.consistencyHash()).toBe(new ConsistencyHash().digest(barrier.shadow));
  });

  it('batches baked keyframes instead of issuing a tiny hash for each key', () => {
    const tree = canonicalProject(large());
    const hash = new ConsistencyHash();
    const start = performance.now();
    hash.digest(tree);
    // Hash work follows the layer/channel count, never the keyframe count.
    expect(hash.hashed).toBeLessThan(96 * 40);
    hash.digest(tree);
    expect(hash.hashed).toBe(0);
    expect(performance.now() - start).toBeLessThan(5000);
  });

  it('does not rehash unchanged branches, including siblings after an order operation', () => {
    const tree = { layers: [{ id: 'a', p: { v: 1 } }, { id: 'b', p: { v: 2 } }] };
    const hash = new ConsistencyHash();
    hash.digest(tree);
    hash.digest(tree);
    expect(hash.hashed).toBe(0);
    expect(hash.reused).toBe(1);
    const ops: PatchOp[] = [{ op: 'order', path: ['layers'], ids: ['b', 'a'] }, { path: ['layers', 0, 'p', 'v'], exists: true, value: 99 }];
    for (const op of ops) { hash.invalidate(tree, [op]); applyOps(tree, [op]); }
    expect(hash.digest(tree)).toBe(new ConsistencyHash().digest(tree));
    expect(hash.reused).toBeGreaterThan(0);
  });

  it('keeps notification cost independent of keyframes and preserves nested custom announcements', async () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
    const doc = installEngineDoc(PM, { mode: parseMode('on'), loadModule: () => module, sink: () => {} });
    PM.proj = large();
    expect(await doc.whenReady).toBe(true);
    const barrier = doc.barrier!;
    const baseline = performance.now();
    diffJsonOps(barrier.shadow, canonicalProject(PM.proj));
    const wholeMs = performance.now() - baseline;
    const batches: any[] = [];
    doc.subscribe(batch => batches.push(batch));
    const started = performance.now();
    for (let i = 0; i < 73; i++) { notifyDocument(PM, 'assets'); PM.doc.flush(); }
    const notificationsMs = performance.now() - started;
    expect(barrier.stats.structuralDiffs).toBe(0);
    expect(notificationsMs).toBeLessThan(Math.max(20, wholeMs));
    // The built-in project → comps relay is a runtime notification too.
    // Publication's exact announcement must not become a full-root diff.
    notifyDocument(PM, 'project'); PM.doc.flush();
    expect(barrier.stats.structuralDiffs).toBe(0);
    // A known metadata write produces a small patch, without the layers.
    PM.proj.assets.one = { id: 'one', kind: 'image', poster: true };
    notifyDocument(PM, 'assets', [['assets', 'one']]); PM.doc.flush();
    expect(batches.at(-1).body.length).toBeLessThan(256);
    expect(decodePatchOps(batches.at(-1).body).every(op => op.path[0] === 'assets')).toBe(true);
    const stop = PM.bus.on('assets', () => { PM.proj.layers[0].p.x.v = 123; PM.touch(); });
    notifyDocument(PM, 'assets'); PM.doc.flush(); stop();
    expect(barrier.shadow.comps.comp.layers[0].p.x.v).toBe(123);
    // Custom nested emitters retain the structural safety net even when a
    // built-in project announcement surrounds them.
    const nested = PM.bus.on('project', () => {
      PM.proj.layers[0].p.x.v = 456; PM.bus.emit('layers');
    });
    notifyDocument(PM, 'project'); PM.doc.flush(); nested();
    expect(barrier.shadow.comps.comp.layers[0].p.x.v).toBe(456);
    console.info(JSON.stringify({ wholeDocumentDiffMs: wholeMs, runtimeNotifications73Ms: notificationsMs, bytes: batches[0]?.body.length }));
  });

  it('diffs an optional composition setting it never had as that member alone, not the whole composition', async () => {
    const instance = await wasm();
    const project: any = large(24, 100);
    const barrier = new DocumentBarrier(instance, () => project);
    barrier.resync();
    const ops: PatchOp[][] = [];
    barrier.subscribe(batch => { if (batch.origin !== 'load') ops.push(decodePatchOps(batch.body)); });
    const path = ['comps', 'comp', 'audioGain'];
    expect(anchorPath(barrier.shadow, canonicalProject({ ...project, audioGain: 2 }), path)).toEqual(path);
    // Under a member that is not an object, the parent is replaced.
    expect(anchorPath({ comps: { comp: 1 } }, { comps: { comp: { audioGain: 2 } } }, path)).toEqual(['comps', 'comp']);
    project.audioGain = 2; barrier.touched([['audioGain']]); barrier.flush();
    delete project.audioGain; barrier.touched([['audioGain']]); barrier.flush();
    // Nothing written: a member neither side has is no operation at all.
    barrier.touched([['shutter']]); barrier.flush();
    expect(ops).toEqual([
      [{ path: ['comps', 'comp', 'audioGain'], exists: true, value: 2 }],
      [{ path: ['comps', 'comp', 'audioGain'], exists: false }],
    ]);
    expect(barrier.check('setting').ok).toBe(true);
  });

  it('keeps the viewer replay base when another consumer takes a snapshot', async () => {
    const instance = await wasm();
    const project = { id: 'shared-consumers', value: 0 };
    const barrier = new DocumentBarrier(instance, () => project);
    barrier.resync();
    const base = barrier.resume();
    project.value = 1; barrier.touched([['value']]); barrier.flush();
    const exportSnapshot = barrier.snapshot();
    expect(exportSnapshot.revision).toBe(base.revision + 1);
    expect(barrier.resume().revision).toBe(base.revision);
    expect(barrier.resume().batches).toHaveLength(1);
    barrier.snapshot({ checkpoint: true });
    expect(barrier.resume().revision).toBe(exportSnapshot.revision);
    expect(barrier.resume().batches).toHaveLength(0);
  });

  it('retains only a bounded contiguous patch suffix and resets it on replacement', async () => {
    const instance = await wasm();
    let project: any = { id: 'journal', value: 0 };
    const barrier = new DocumentBarrier(instance, () => project);
    barrier.resync();
    const base = barrier.resume();
    for (let i = 1; i <= 3; i++) { project.value = i; barrier.touched([['value']]); barrier.flush(); }
    const resume = barrier.resume();
    expect(resume.revision).toBe(base.revision);
    expect(resume.batches.map(batch => batch.baseRevision)).toEqual([base.revision, base.revision + 1, base.revision + 2]);
    for (let i = 4; i <= 132; i++) { project.value = i; barrier.touched([['value']]); barrier.flush(); }
    const bounded = barrier.resume();
    expect(bounded.revision).toBe(barrier.revision);
    expect(bounded.batches).toHaveLength(0);
    project = { id: 'other', value: 999 }; barrier.resync();
    expect(barrier.resume().hash).not.toBe(base.hash);
    expect(barrier.resume().batches).toHaveLength(0);
  });
});
