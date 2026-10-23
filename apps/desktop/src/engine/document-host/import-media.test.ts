import { describe, expect, it, vi } from 'vitest';
import { installWorkerImports } from './import-media';

/** The worker's import_media/replace_media: today's editor import runs in the
 *  host's tab; the worker answers only once that commit is in its document. */
describe('document host media imports', () => {
  const setup = (commit: (PM: any) => void) => {
    const PM: any = { proj: { id: 'P', revision: 3, assets: {} } };
    const run = vi.fn(async (_project: string, _args: Record<string, unknown>, _replace: boolean) => {
      commit(PM);
      return { status: 'imported', assetId: 'a1', name: 'tone.wav', persisted: true, fingerprint: 'v2:1:f', storage: 'copied', revision: 9 };
    });
    installWorkerImports(PM, run, () => 'P');
    return { PM, run };
  };

  it('runs today\'s import in the host tab and answers with the committed record at this document\'s revision', async () => {
    const { PM, run } = setup(PM => { PM.proj = { ...PM.proj, revision: 4, assets: { a1: { id: 'a1', fingerprint: 'v2:1:f' } } }; });
    const assertCurrent = vi.fn();
    const result = await PM.HostImportMedia({ path: '/media/tone.wav', folderId: 'f1' }, false, assertCurrent);
    expect(run).toHaveBeenCalledWith('P', { path: '/media/tone.wav', folderId: 'f1' }, false);
    expect(assertCurrent).toHaveBeenCalledTimes(1);
    expect(result).toEqual({ status: 'imported', assetId: 'a1', name: 'tone.wav', persisted: true, fingerprint: 'v2:1:f', storage: 'copied', revision: 4 });
  });

  it('names a commit that never reached the worker instead of answering as imported', async () => {
    const missing = setup(() => undefined);
    await expect(missing.PM.HostImportMedia({ path: '/media/tone.wav' }, false, () => undefined)).rejects.toThrow(/^media-import-unsynced/);
    const stale = setup(PM => { PM.proj.assets.a1 = { id: 'a1', fingerprint: 'v2:1:old' }; });
    await expect(stale.PM.HostImportMedia({ path: '/media/tone.wav' }, true, () => undefined)).rejects.toThrow(/^media-import-unsynced/);
  });

  it('never starts an import the run no longer owns', async () => {
    const { PM, run } = setup(() => undefined);
    await expect(PM.HostImportMedia({ path: '/media/tone.wav' }, false, () => { throw new Error('The project changed.'); })).rejects.toThrow('The project changed.');
    expect(run).not.toHaveBeenCalled();
  });
});
