import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { canonicalProjectHash } from '../../../shared/canonical-json';
import { makePM } from '../legacy/__tests__/make-pm';
import { parseMode } from './flags';
import { installEngineDoc } from './install';
import { createTracker, isTrackingProxy, patchCloning } from './proxy';

describe('the write-tracking Proxy', () => {
  const tracked = (mode: 'report' | 'throw' = 'report', writer = { on: false }) => {
    const writes: string[] = [];
    const tracker = createTracker({ mode, inWriter: () => writer.on, onWrite: path => writes.push(path.join('.')) });
    const raw = { layers: [{ id: 'a', p: { x: { v: 1, kf: [] } } }, { id: 'b', p: {} }], name: 'P' };
    return { tracker, raw, doc: tracker.wrap(raw), writes, writer };
  };

  it('records the path of every write; array structure records the array', () => {
    const { doc, writes } = tracked();
    doc.layers[0].p.x.v = 5;
    doc.name = 'Q';
    doc.layers.push({ id: 'c', p: {} });
    doc.layers.splice(0, 1);
    delete doc.layers[0].p;
    expect(writes).toContain('layers.0.p.x.v');
    expect(writes).toContain('name');
    expect(writes).toContain('layers');
    expect(writes).toContain('layers.0.p');
  });

  it('reports writes outside a known writer, and throws in strict mode', () => {
    const report = tracked('report');
    report.doc.name = 'x';
    expect(report.tracker.violations()).toEqual([expect.objectContaining({ path: 'name', count: 1 })]);
    // The site is the writing code, not the Proxy's own frames.
    expect(report.tracker.violations()[0]!.site).toMatch(/proxy\.test\.ts/);
    report.writer.on = true;
    report.doc.name = 'y';
    expect(report.tracker.violations()[0]!.count).toBe(1);
    const strict = tracked('throw');
    expect(() => { strict.doc.name = 'x'; }).toThrow(/outside a known writer/);
  });

  it('keeps the raw document free of proxies and identity checks working', () => {
    const { doc, raw, tracker } = tracked('report', { on: true });
    const layer = doc.layers[0];
    expect(isTrackingProxy(layer)).toBe(true);
    doc.layers.push({ id: 'c', copy: layer, p: {} });
    expect(isTrackingProxy(raw.layers[2])).toBe(false);
    expect(isTrackingProxy((raw.layers[2] as any).copy)).toBe(false);
    expect((raw.layers[2] as any).copy).toBe(raw.layers[0]);
    expect(doc.layers.indexOf(raw.layers[1])).toBe(1);
    expect(doc.layers.includes(layer)).toBe(true);
    expect(tracker.unwrap(layer)).toBe(raw.layers[0]);
  });

  it('hands out objects that entered the document in this task raw, and proxies after it', async () => {
    const { doc, tracker, writes } = tracked('report', { on: true });
    const mask = { id: 'm', path: { vertices: [] as unknown[] } };
    doc.layers[0].masks = [];
    doc.layers[0].masks.push(mask);
    // The inserting code compares its own reference with what it reads back.
    expect(doc.layers[0].masks.find((m: any) => m.path === mask.path)).toBe(mask);
    expect(isTrackingProxy(doc.layers[0].masks[0])).toBe(false);
    // Objects read through the Proxy before stay proxies.
    expect(isTrackingProxy(doc.layers[1])).toBe(true);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(isTrackingProxy(doc.layers[0].masks[0])).toBe(true);
    writes.length = 0;
    doc.layers[0].masks[0].path.vertices.push(1);
    expect(new Set(writes)).toEqual(new Set(['layers.0.masks.0.path.vertices']));
    tracker.settle();
  });

  it('keeps proxies out of the raw document when they are stored through a raw reference', () => {
    const { doc, raw, tracker } = tracked('report', { on: true });
    const layer = doc.layers[0];
    // Code that copies members of the document into a raw container.
    const copy: any = { ...raw, layers: [layer] };
    (raw as any).copy = { layers: doc.layers };
    expect(isTrackingProxy((raw as any).copy.layers)).toBe(true);
    expect(isTrackingProxy(doc.copy.layers)).toBe(true);
    expect(isTrackingProxy((raw as any).copy.layers)).toBe(false);
    expect(isTrackingProxy(tracker.unwrapDeep(copy).layers[0])).toBe(false);
  });

  it('lets structuredClone and postMessage take proxies', () => {
    const { doc, tracker } = tracked();
    const scope: any = { structuredClone: globalThis.structuredClone };
    const restore = patchCloning(tracker, scope);
    expect(scope.structuredClone(doc.layers[0])).toEqual({ id: 'a', p: { x: { v: 1, kf: [] } } });
    expect(scope.structuredClone({ wrapped: doc.layers[1] })).toEqual({ wrapped: { id: 'b', p: {} } });
    restore();
  });
});

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));

describe.skipIf(!existsSync(MODULE))('tracking mode in the editor', () => {
  it('keeps the mirror current from the Proxy alone and reports unclassified writers', async () => {
    const lines: string[] = [];
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
    const doc = installEngineDoc(PM, { mode: parseMode('audit-proxy'), loadModule: () => WebAssembly.compile(readFileSync(MODULE)), sink: l => lines.push(l), auditMs: 1e9 });
    PM.proj = PM.mkProject({ dur: 5 });
    expect(await doc.whenReady).toBe(true);
    doc.auditor!.stop();
    const layer = PM.mkLayer('solid');
    PM.hist.do('Add', () => PM.addLayer(layer));
    expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'position.x', value: 321 }).ok).toBe(true);
    PM.hist.undo();
    PM.hist.redo();
    expect(doc.tracker!.violations()).toEqual([]);
    // A direct write with no touch(): the Proxy saw it (and reports the writer).
    // (The layer entered the document in this task: reads hand it out raw
    // until the task ends.)
    doc.tracker!.settle();
    PM.proj.layers[0].name = 'Direct';
    expect(doc.tracker!.violations().map(v => v.path)).toEqual(['layers.0.name']);
    doc.barrier!.flush();
    expect(doc.wasm!.hash()).toBe(canonicalProjectHash(PM.proj));
    expect(doc.auditor!.audit()).toBe(true);
    expect(doc.dual!.stats.samples).toEqual([]);
    expect(doc.api!.worldMatrix(PM.proj.layers[0], 0)).toEqual(PM.worldMatrix(PM.proj.layers[0], 0));
  });
});
