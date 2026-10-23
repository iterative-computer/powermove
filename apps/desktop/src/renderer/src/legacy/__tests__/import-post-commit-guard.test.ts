import { afterEach, describe, expect, it, vi } from 'vitest';

import { makePM } from './make-pm';

/* After an import commits, only the project/epoch guard runs across the host
 * media binding flush. The caller's own `assertCurrent` (cancel, deadline,
 * revision, history mark) is enforced before the commit; after it the
 * import's own write has moved the revision, so re-running it would fail a
 * successful import. */
function setup(flush?: (project: unknown) => Promise<void>) {
  vi.stubGlobal('window', {
    document: { createElement: () => ({ getContext: () => ({}) }) },
    navigator: { hardwareConcurrency: 4 },
    URL: { createObjectURL: () => 'blob:test', revokeObjectURL() {} },
    setTimeout, clearTimeout,
  });
  const project = { id: 'A', assets: {} as Record<string, unknown>, layers: [] };
  const PM = makePM('gl/raster');
  PM.proj = project;
  PM.uid = () => 'asset-new';
  PM.Audio = {
    accepts: () => true,
    prepareAsset: async () => ({ id: 'asset-new', name: 'clip.mp3', kind: 'audio' }),
    disposeAsset() {},
    rebalanceCache() {},
  };
  PM.MediaStore = { put: async () => true };
  PM.MediaImport = {
    fingerprint: async () => 'fingerprint',
    storageKeyFor: (value: string) => `media:${value}`,
    match: () => ({ canonicalId: null, aliases: [] }),
    coalesce: () => 0,
    mapBounded: async (items: unknown[], _limit: number, worker: (item: unknown) => unknown) => Promise.all(items.map(worker)),
  };
  PM.touch = () => {};
  if (flush) PM.HostMedia = { flushBindings: flush };
  return { PM, project };
}

afterEach(() => vi.unstubAllGlobals());

describe('import post-commit guard', () => {
  it('returns a committed import even though its own write moved the caller revision', async () => {
    const { PM, project } = setup();
    const checks: boolean[] = [];
    const assertCurrent = () => {
      const committed = Object.keys(project.assets).length > 0;
      checks.push(committed);
      if (committed) throw new Error('revision moved');
    };
    const asset = await PM.assets.add({ name: 'clip.mp3', type: 'audio/mpeg', size: 10 }, { silent: true, assertCurrent });
    expect(asset.importResult.status).toBe('created');
    expect(Object.keys(project.assets)).toEqual(['asset-new']);
    expect(checks.length).toBeGreaterThan(0);
    expect(checks.every(committed => !committed)).toBe(true);
  });

  it('still names a project switch during the binding flush', async () => {
    const other = { id: 'B', assets: {}, layers: [] };
    let PM: any;
    ({ PM } = setup(async () => { PM.proj = other; }));
    await expect(PM.assets.add({ name: 'clip.mp3', type: 'audio/mpeg', size: 10 }, { silent: true }))
      .rejects.toMatchObject({ code: 'STALE_MEDIA_IMPORT', message: expect.stringMatching(/switched projects · inspect the imported media/) });
  });
});
