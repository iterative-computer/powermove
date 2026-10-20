import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { decodeProjectContainer } from '../src/shared/project-container';
import { INCREMENTAL_MAGIC, projectFooter } from '../src/shared/project-incremental';
import { expect, test } from './helpers/app';

test('a large native document validates outside main and remains editable, undoable, and saveable', async ({ session }, info) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const project = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'add_layer', layerType: 'solid', name: 'Editable source', content: { w: 120, h: 80, color: '#FF7733' } });
    return JSON.parse(PM.serialize()).proj;
  });
  const document = Buffer.from(JSON.stringify({ proj: { ...project, name: 'Large validation proof' },
    history: { version: 1, index: -1, entries: [] }, memoryFixture: 'x'.repeat(64 * 1024 * 1024) }));
  const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
  const index = Buffer.from(JSON.stringify({ version: 4,
    document: { offset: INCREMENTAL_MAGIC.length, length: document.length, sha256: hash(document) }, media: [] }));
  const file = path.join(session.userData, 'Large validation proof.pmv');
  await writeFile(file, Buffer.concat([INCREMENTAL_MAGIC, document, index,
    projectFooter(INCREMENTAL_MAGIC.length + document.length, index.length, hash(index))]));
  await session.app.evaluate(({ dialog }, file) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [file] });
    dialog.showSaveDialog = async () => { throw new Error('An opened file must reuse its granted destination'); };
    const workers = process.getBuiltinModule('node:worker_threads');
    const proof = (globalThis as any).__projectWorkerProof = { parse: JSON.parse, Worker: workers.Worker,
      largeParses: 0, started: 0, exited: 0, ticks: 0, gap: 0, last: performance.now(), timer: null };
    JSON.parse = (text, ...args) => {
      if (text.length >= 64 * 1024 * 1024) proof.largeParses++;
      return proof.parse(text, ...args);
    };
    workers.Worker = class extends proof.Worker {
      constructor(source, options) {
        super(source, options);
        if (options?.name === 'Powermove project validation') {
          proof.started++;
          this.once('exit', () => proof.exited++);
        }
      }
    };
    proof.timer = setInterval(() => {
      const now = performance.now();
      proof.gap = Math.max(proof.gap, now - proof.last); proof.last = now; proof.ticks++;
    }, 1);
  }, file);
  try {
    await session.page.evaluate(() => (window as any).PM.openProject());
    const proof = await session.app.evaluate(() => {
      const proof = (globalThis as any).__projectWorkerProof;
      clearInterval(proof.timer);
      return { largeParses: proof.largeParses, started: proof.started, exited: proof.exited,
        ticks: proof.ticks, longestMainTimerGapMs: proof.gap };
    });
    await info.attach('large-document-validation', { body: JSON.stringify(proof), contentType: 'application/json' });
    // This also proves a silent native-parser fallback did not pass the test.
    expect(proof.largeParses).toBe(0);
    expect(proof.started).toBe(1);
    expect(proof.exited).toBe(1);
    expect(proof.ticks).toBeGreaterThan(5);
    expect(await session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Large validation proof');
    const edits = await session.page.evaluate(() => {
      const PM = (window as any).PM, layer = PM.proj.layers[0];
      const result = PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { name: 'Changed source' } });
      const changed = PM.proj.layers[0].name, undone = PM.hist.undo(), restored = PM.proj.layers[0].name;
      const redone = PM.hist.redo();
      return { ok: result.ok, changed, undone, restored, redone, final: PM.proj.layers[0].name };
    });
    expect(edits).toEqual({ ok: true, changed: 'Changed source', undone: true, restored: 'Editable source',
      redone: true, final: 'Changed source' });
    expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    const saved = decodeProjectContainer(await readFile(file)).document as any;
    expect(saved.proj.name).toBe('Large validation proof');
    expect(saved.proj.layers[0].name).toBe('Changed source');
    await session.page.evaluate(() => (window as any).PM.openProject());
    expect(await session.page.evaluate(() => (window as any).PM.proj.layers[0].name)).toBe('Changed source');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.app.evaluate(() => {
      const proof = (globalThis as any).__projectWorkerProof;
      clearInterval(proof.timer); JSON.parse = proof.parse;
      process.getBuiltinModule('node:worker_threads').Worker = proof.Worker;
      delete (globalThis as any).__projectWorkerProof;
    });
  }
});
