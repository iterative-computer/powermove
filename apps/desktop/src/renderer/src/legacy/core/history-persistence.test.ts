import { expect, it, vi } from 'vitest';
import { install } from './history';
import { packProjectFile, restoreProjectFileMedia, unpackProjectFile } from './project-file';

function editor(proj: any = { id: 'p', value: 1, layers: [] }) {
  const PM: any = { proj, sel: {}, uid: () => Math.random().toString(),
    replaceProject: (next: any) => { PM.proj = next; }, bus: { emit: vi.fn() },
    touch: vi.fn(), invalidate: vi.fn(), autosave: vi.fn() };
  install(PM);
  return PM;
}

it('keeps both sides of the history cursor through a saved file and fresh editor', async () => {
  const PM = editor();
  PM.hist.do('Two', () => { PM.proj.value = 2; });
  PM.hist.do('Three', () => { PM.proj.value = 3; });
  PM.hist.undo();
  const data = await packProjectFile({ proj: PM.proj, history: PM.hist.export() }, { get: async () => null, put: async () => true });
  const saved = unpackProjectFile(data);
  const reopened = editor(saved.proj);
  reopened.hist.import(saved.history);
  expect(reopened.hist.canUndo()).toBe(true);
  expect(reopened.hist.canRedo()).toBe(true);
  reopened.hist.undo(); expect(reopened.proj.value).toBe(1);
  reopened.hist.redo(); expect(reopened.proj.value).toBe(2);
  reopened.hist.redo(); expect(reopened.proj.value).toBe(3);
});

it('retains grouped edits and drops the redo branch after a new edit', () => {
  const PM = editor();
  const mark = PM.hist.mark();
  PM.hist.do('Two', () => { PM.proj.value = 2; });
  PM.hist.do('Three', () => { PM.proj.value = 3; });
  PM.hist.squash(mark, 'Agent change');
  const next = editor(JSON.parse(JSON.stringify(PM.proj)));
  next.hist.import(PM.hist.export());
  expect(next.hist.list()).toEqual(['Agent change']);
  next.hist.undo(); expect(next.proj.value).toBe(1);
  next.hist.redo(); expect(next.proj.value).toBe(3);
  next.hist.undo();
  next.hist.do('Four', () => { next.proj.value = 4; });
  expect(next.hist.canRedo()).toBe(false);
});

it('clears history for old files and rejects unsafe patch paths', () => {
  const PM = editor();
  PM.hist.do('Two', () => { PM.proj.value = 2; });
  PM.hist.import(undefined);
  expect(PM.hist.canUndo()).toBe(false);
  PM.hist.import({ version: 1, index: 0, entries: [{ label: 'Bad', forward: [{ path: ['__proto__', 'polluted'], exists: true, value: true }], backward: [] }] });
  expect(PM.hist.canUndo()).toBe(false);
  expect(({} as any).polluted).toBeUndefined();
});

it('embeds media that exists only in an undo step', async () => {
  const PM = editor();
  PM.proj.assets = { image: { id: 'image', name: 'image.png' } };
  PM.hist.do('Delete image', () => { delete PM.proj.assets.image; });
  const get = vi.fn(async () => new Blob(['pixels']));
  const put = vi.fn(async () => true);
  const saved = unpackProjectFile(await packProjectFile({ proj: PM.proj, history: PM.hist.export() }, { get, put }));
  expect(get).toHaveBeenCalledOnce();
  await restoreProjectFileMedia(saved, { get, put });
  expect(put).toHaveBeenCalledWith('image', expect.any(Blob), { id: 'image', name: 'image.png' });
});
