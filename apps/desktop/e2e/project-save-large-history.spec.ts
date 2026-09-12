import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

test('large undo history survives autosave, native Save, and relaunch', async ({ session }) => {
  test.setTimeout(90_000);
  const destination = path.join(session.userData, 'Large history.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, destination);
  const projectId = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Large history' }),
    }));
    PM.hist.clear();
    // A deleted source remains in undo history even when the project is small.
    PM.proj.historyTestSource = 'x'.repeat(33792768);
    PM.hist.do('Delete large source', () => { delete PM.proj.historyTestSource; });
    return PM.proj.id as string;
  });
  const statePath = path.join(session.userData, 'store', `projectHistory.${projectId}.json`);
  await expect.poll(async () => (await stat(statePath).catch(() => null))?.size || 0)
    .toBeGreaterThan(32 * 1024 * 1024);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  const saved = decodeProjectContainer(new Uint8Array(await readFile(destination))).document as any;
  expect(saved.history.entries[0].label).toBe('Delete large source');
  expect(await session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(false);

  await session.relaunch();
  const restored = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const id = PM.proj.id;
    const undone = PM.hist.undo();
    const length = PM.proj.historyTestSource?.length;
    const redone = PM.hist.redo();
    return { id, undone, length, redone, deleted: !('historyTestSource' in PM.proj) };
  });
  expect(restored).toEqual({ id: projectId, undone: true, length: 33792768, redone: true, deleted: true });
  expect(session.diagnostics.console.filter(record => /Could not save|Project save failed|serialised value/.test(record.text))).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
