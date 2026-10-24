import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

test('saving an inactive tab preserves live edits after a local recovery write fails', async ({ session }) => {
  await session.openEditor();
  const destination = path.join(session.userData, 'Inactive.pmv');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, destination);
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.flushProject();
    const first = PM.proj.id;
    PM.newBlankProject({ name: 'Second document' });
    const second = PM.proj.id;
    const set = PM.store.set;
    // Simulate a full/unavailable recovery store while keeping the external
    // project destination writable. All edit and native save code still runs.
    PM.store.set = (key: string, value: any) => key === `projectJournal.${second}` ? false : set(key, value);
    // The native engine branch writes the journal off the edit's task (setAsync).
    const setAsync = PM.store.setAsync;
    if (setAsync) PM.store.setAsync = (key: string, value: any) => key === `projectJournal.${second}` ? Promise.resolve(false) : setAsync(key, value);
    // With main keeping the journal (SAVE-1), entries are appended there instead.
    const appendJournal = PM.store.appendJournal;
    if (appendJournal) PM.store.appendJournal = (key: string, value: any) => key === `projectJournal.${second}` ? Promise.reject(new Error('The recovery store is full')) : appendJournal(key, value);
    PM.hist.do('Rename second', () => { PM.proj.name = 'Latest second document'; });
    const live = PM.proj;
    const recover = PM.Projects.recoverAsync;
    let release!: () => void, started = false;
    const pause = new Promise<void>(resolve => { release = resolve; });
    PM.Projects.recoverAsync = async (...args: any[]) => { started = true; await pause; return recover(...args); };
    try {
      if (!await PM.Tabs.activate(first)) throw new Error('Could not switch to first tab');
      const recoveryName = PM.Projects.get(second).name;
      const saved = await PM.saveProject({ projectId: second });
      const dirty = PM.projectFileState(second).dirty, activeStayed = PM.proj.id === first;
      if (!await PM.Tabs.activate(second)) throw new Error('Could not resume second tab');
      const sameDocument = PM.proj === live;
      const liveName = PM.proj.name;
      const undo = PM.hist.undo(), restoredName = PM.proj.name;
      return { started, recoveryName, saved, dirty, activeStayed, sameDocument, liveName, undo, restoredName };
    } finally { PM.store.set = set; if (setAsync) PM.store.setAsync = setAsync; if (appendJournal) PM.store.appendJournal = appendJournal; PM.Projects.recoverAsync = recover; release(); }
  });
  expect(result).toEqual({ started: true, recoveryName: 'Second document', saved: true, dirty: false,
    activeStayed: true, sameDocument: true, liveName: 'Latest second document', undo: true, restoredName: 'Second document' });
  const saved = decodeProjectContainer(await readFile(destination)).document;
  expect(saved.proj.name).toBe('Latest second document');
  expect(saved.history.entries.map((entry: any) => entry.label)).toEqual(['Rename second']);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
