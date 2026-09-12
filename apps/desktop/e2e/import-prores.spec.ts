import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.describe('@import-prores automatic playback proxy', () => {
  test('converts ProRes once, presents video frames, and restores the durable proxy', async ({ session }) => {
    await importFixture(session.page, 'prores422.mov');
    await session.page.waitForFunction(() => {
      const PM = (window as any).PM;
      return [...PM.assets.map.values()].some((asset: any) => asset.name === 'prores422.mov');
    });

    const projectId = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      const asset = [...PM.assets.map.values()].find((candidate: any) => candidate.name === 'prores422.mov') as any;
      const layer = PM.proj.layers.find((candidate: any) => candidate.d?.asset === asset.id);
      PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: 0 } });
      PM.setTime(0, { raw: true, force: true });
      let frames = 0;
      const count = () => { frames++; asset.el.requestVideoFrameCallback(count); };
      asset.el.requestVideoFrameCallback(count);
      PM.play();
      await new Promise(resolve => setTimeout(resolve, 900));
      PM.pause();
      if (!asset.playbackProxy || !PM.proj.assets[asset.id]?.playbackProxy || frames < 10) {
        throw new Error(JSON.stringify({ liveProxy: asset.playbackProxy, durableProxy: PM.proj.assets[asset.id]?.playbackProxy, frames }));
      }
      await PM.flushProject();
      return PM.proj.id;
    });

    await session.relaunch();
    await session.page.waitForFunction((id) => (window as any).PM.proj.id === id, projectId);
    const restored = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      const asset = [...PM.assets.map.values()].find((candidate: any) => candidate.name === 'prores422.mov') as any;
      const stored = await PM.MediaStore.get(PM.proj.assets[asset.id]);
      let frames = 0;
      const count = () => { frames++; asset.el.requestVideoFrameCallback(count); };
      asset.el.requestVideoFrameCallback(count);
      PM.setTime(0, { raw: true, force: true });
      PM.play();
      await new Promise(resolve => setTimeout(resolve, 700));
      PM.pause();
      return { playbackProxy: asset.playbackProxy, storedBytes: stored?.size || 0, frames };
    });

    expect(restored.playbackProxy).toBe(true);
    expect(restored.storedBytes).toBeGreaterThan(0);
    expect(restored.frames).toBeGreaterThan(8);
  });
});
