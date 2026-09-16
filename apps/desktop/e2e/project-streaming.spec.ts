import { open, stat, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

// A disk fixture, not a giant JS buffer. History-only media exercises both
// portable storage paths without asking an image/video decoder to parse padding.
test('opens and saves media beyond 1 GiB with bounded main-process memory', async ({ session }) => {
  test.setTimeout(240000);
  await session.openEditor();
  const source = path.join(session.userData, 'Source.pmv');
  const destination = path.join(session.userData, 'Streamed.pmv');
  const length = 1024 ** 3 + 17;
  const asset = { id: 'large-source', kind: 'video', type: 'video/webm', name: 'Retained by undo.webm', size: length };
  const project = await session.page.evaluate(() => JSON.parse((window as any).PM.serialize()).proj);
  const header = Buffer.from(JSON.stringify({
    document: { proj: { ...project, name: 'Large streamed project', layers: [], assets: {} },
      history: { version: 1, index: 0, entries: [{ label: 'Delete media',
        forward: [{ path: ['assets', asset.id], exists: false }],
        backward: [{ path: ['assets', asset.id], exists: true, value: asset }] }] } },
    media: [{ id: asset.id, type: asset.type, offset: 0, length }]
  }));
  const prefix = Buffer.alloc(9); prefix.write('PMV3\n'); prefix.writeUInt32LE(header.length, 5);
  const file = await open(source, 'w');
  try {
    await file.writeFile(Buffer.concat([prefix, header]));
    await file.truncate(9 + header.length + length);
    await file.write(Buffer.from([7, 8, 9, 10]), 0, 4, 9 + header.length + length - 4);
  } finally { await file.close(); }
  await session.app.evaluate(({ dialog }, paths) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [paths.source] });
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: paths.destination });
    const state = (globalThis as any).__streamMemory = { base: process.memoryUsage().external, peak: 0 };
    state.timer = setInterval(() => { state.peak = Math.max(state.peak, process.memoryUsage().external); }, 10);
  }, { source, destination });
  await session.page.evaluate(() => (window as any).PM.openProject());
  const restored = await session.page.evaluate(async asset => {
    const PM = (window as any).PM, blob = await PM.MediaStore.get(asset);
    return { name: PM.proj.name, bytes: blob?.size,
      tail: blob ? [...new Uint8Array(await blob.slice(-4).arrayBuffer())] : [],
      history: PM.hist.export().entries.length,
      staging: await (async () => { const names = []; for await (const name of (await navigator.storage.getDirectory() as any).keys()) names.push(name); return names; })() };
  }, asset);
  expect(restored).toMatchObject({ name: 'Large streamed project', bytes: length, tail: [7, 8, 9, 10], history: 1 });
  expect(restored.staging.filter((name: string) => name.startsWith('project-import-'))).toEqual([]);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject({ saveAs: true }))).toBe(true);
  const size = (await stat(destination)).size;
  expect(size).toBeGreaterThan(length);
  const saved = await open(destination, 'r');
  try {
    const tail = Buffer.alloc(4); await saved.read(tail, 0, 4, size - 4);
    expect([...tail]).toEqual([7, 8, 9, 10]);
  } finally { await saved.close(); }
  const memory = await session.app.evaluate(() => {
    const state = (globalThis as any).__streamMemory; clearInterval(state.timer);
    return { base: state.base, peak: state.peak, growthMiB: (state.peak - state.base) / 1024 ** 2 };
  });
  console.log('Streamed >1 GiB project main-process buffer growth:', memory);
  expect(memory.growthMiB).toBeLessThan(192);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('portable Export uses native streaming and honors a cancelled file picker', async ({ session }) => {
  await session.openEditor();
  const destination = path.join(session.userData, 'Export.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, destination);
  expect(await session.page.evaluate(() => (window as any).PM.Export.run({ format: 'json' }))).toEqual({ cancelled: false });
  expect(decodeProjectContainer(await readFile(destination)).document.proj.layers).toEqual([]);
  await session.app.evaluate(({ dialog }) => { dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' }); });
  expect(await session.page.evaluate(() => (window as any).PM.Export.run({ format: 'json' }))).toEqual({ cancelled: true });
});
