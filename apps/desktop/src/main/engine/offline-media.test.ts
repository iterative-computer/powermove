import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({ dialog: { showMessageBox: vi.fn() } }));

import { askAboutOfflineMedia, needsBlender, offlineClips, offlineExportOptions, offlineMessage, offlineTitle, referencedClips } from './offline-media';

const layer = (id: string, type: string, d: Record<string, unknown>) => ({ id, type, d });

const project = () => ({
  compId: 'root',
  layers: [
    layer('v1', 'video', { asset: 'a1' }),
    layer('p1', 'precomp', { comp: 'inner' }),
    layer('t1', 'text', { text: 'x' }),
    layer('ghost', 'video', { asset: 'missing-record' }),
    layer('v1b', 'video', { asset: 'a1' })
  ],
  comps: {
    inner: { layers: [layer('i1', 'image', { asset: 'a2' }), layer('s1', 'audio', { asset: 'a3' }), layer('self', 'precomp', { comp: 'root' })] },
    unused: { layers: [layer('u1', 'video', { asset: 'a4' })] }
  },
  assets: {
    a1: { name: 'A001_C003.mov' }, a2: { name: 'logo.png' }, a3: { name: '' }, a4: { name: 'unused.mov' }
  }
});

describe('offline media at export', () => {
  it('lists the clips a composition reaches, once each, through nested compositions', () => {
    expect(referencedClips(project())).toEqual([
      { asset: 'a1', name: 'A001_C003.mov' },
      { asset: 'a2', name: 'logo.png' },
      { asset: 'a3', name: 'a3' }
    ]);
  });

  it('reads legacy root compositions without a compId', () => {
    const p: any = project();
    p.id = p.compId; delete p.compId;
    expect(referencedClips(p).map(clip => clip.asset)).toEqual(['a1', 'a2', 'a3']);
  });

  it('finds the offline ones through the media store', async () => {
    const online = new Set(['a1']);
    const clips = await offlineClips(project(), async (asset) => (online.has(asset) ? { path: '/x' } : null));
    expect(clips.map((c) => c.asset)).toEqual(['a2', 'a3']);
  });

  it('offers Locate, Export Without Missing Media, and Cancel', async () => {
    const clips = [{ asset: 'a', name: 'A.mov' }, { asset: 'b', name: 'B.mov' }];
    expect(offlineTitle(2)).toBe('2 clips are offline');
    expect(offlineTitle(1)).toBe('1 clip is offline');
    const four = offlineMessage(clips, true);
    expect(four).toMatchObject({
      message: '2 clips are offline',
      buttons: ['Locate Media…', 'Export with Proxies', 'Export Without Missing Media', 'Cancel'],
      defaultId: 0,
      cancelId: 3
    });
    expect(four.detail).toContain('export without the missing clips');
    expect(offlineMessage(clips, false)).toMatchObject({ buttons: ['Locate Media…', 'Export Without Missing Media', 'Cancel'], cancelId: 2 });
    const window = {} as never;
    const show = (response: number) => vi.fn(async () => ({ response, checkboxChecked: false }));
    expect(await askAboutOfflineMedia(window, clips, true, show(0))).toBe('locate');
    expect(await askAboutOfflineMedia(window, clips, true, show(1))).toBe('proxies');
    expect(await askAboutOfflineMedia(window, clips, true, show(2))).toBe('without');
    expect(await askAboutOfflineMedia(window, clips, true, show(3))).toBe('cancel');
    expect(await askAboutOfflineMedia(window, clips, false, show(1))).toBe('without');
    expect(await askAboutOfflineMedia(window, clips, false, show(2))).toBe('cancel');
    expect(offlineExportOptions({ format: 'png', draft: true }, 'without')).toEqual({ format: 'png', draft: false });
    expect(offlineExportOptions({ format: 'png', draft: false }, 'proxies')).toEqual({ format: 'png', draft: true });
  });

  it('knows when a composition renders with Blender', () => {
    expect(needsBlender(project())).toBe(false);
    const p = project() as any;
    p.comps.inner.render3d = { enabled: true };
    expect(needsBlender(p)).toBe(true);
  });
});
