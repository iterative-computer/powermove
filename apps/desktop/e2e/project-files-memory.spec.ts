import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { INCREMENTAL_MAGIC, projectFooter } from '../src/shared/project-incremental';
import { expect, test } from './helpers/app';

test('opening a large native project releases its main-process document after import', async ({ session }) => {
  await session.openEditor();
  const project = await session.page.evaluate(() => JSON.parse((window as any).PM.serialize()).proj);
  const document = Buffer.from(JSON.stringify({ proj: { ...project, name: 'Memory proof' },
    history: { version: 1, index: -1, entries: [] }, memoryFixture: 'x'.repeat(4 * 1024 * 1024) }));
  const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
  const index = Buffer.from(JSON.stringify({ version: 4,
    document: { offset: INCREMENTAL_MAGIC.length, length: document.length, sha256: hash(document) }, media: [] }));
  const file = path.join(session.userData, 'Memory proof.pmv');
  await writeFile(file, Buffer.concat([INCREMENTAL_MAGIC, document, index,
    projectFooter(INCREMENTAL_MAGIC.length + document.length, index.length, hash(index))]));
  await session.app.evaluate(({ dialog }, file) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [file] });
    const original = JSON.parse;
    // Observe the real native open, not another instance of ProjectFiles.
    // This instrumentation stays inside the disposable regression process.
    const proof = (globalThis as any).__projectDocumentMemory = { original, weak: null as WeakRef<object> | null,
      collect: null as (() => void) | null };
    JSON.parse = (text, ...args) => {
      const value = original(text, ...args);
      if (value?.proj?.name === 'Memory proof' && value.memoryFixture?.length === 4 * 1024 * 1024) proof.weak = new WeakRef(value);
      return value;
    };
    process.getBuiltinModule('node:v8').setFlagsFromString('--expose_gc');
    proof.collect = process.getBuiltinModule('node:vm').runInNewContext('gc');
  }, file);
  try {
    await session.page.evaluate(() => (window as any).PM.openProject());
    expect(await session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Memory proof');
    expect(await session.app.evaluate(() => {
      const proof = (globalThis as any).__projectDocumentMemory;
      JSON.parse = proof.original;
      return !!proof.weak;
    })).toBe(true);
    await expect.poll(() => session.app.evaluate(() => {
      const proof = (globalThis as any).__projectDocumentMemory;
      proof.collect();
      return !!proof.weak.deref();
    }), { timeout: 10_000 }).toBe(false);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.app.evaluate(() => {
      const proof = (globalThis as any).__projectDocumentMemory;
      JSON.parse = proof.original; delete (globalThis as any).__projectDocumentMemory;
    });
  }
});
