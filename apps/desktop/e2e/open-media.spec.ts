import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';

test.describe('Open media from Home', () => {
  for (const file of ['h264-aac.mp4', 'still-red.png']) {
    test(`matches the source settings for ${file}`, async ({ session }) => {
      await session.openEditor();
      await session.page.evaluate(() => {
        const PM = (window as any).PM;
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ w: 1920, h: 1080, fps: 60, dur: 10 }) }));
        PM.ProjectsScreen.show();
      });
      const chooser = session.page.waitForEvent('filechooser');
      await session.page.getByRole('button', { name: 'Open…', exact: true }).click();
      await (await chooser).setFiles(fixturePath(file));
      await session.page.waitForFunction(name => (window as any).PM.proj.layers.some((l: any) => l.name === name), file);
      const state = await session.page.evaluate(name => {
        const PM = (window as any).PM;
        const asset = [...PM.assets.map.values()].find((a: any) => a.name === name) as any;
        const layer = PM.proj.layers.find((l: any) => l.name === name);
        return { w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: PM.proj.dur, work: PM.proj.work,
          exportFps: PM.proj.exportDefaults.fps, source: { w: asset.w, h: asset.h, dur: asset.dur },
          layer: { from: layer.from, dur: layer.dur, w: layer.d.w, h: layer.d.h } };
      }, file);
      expect(state.w).toBe(state.source.w);
      expect(state.h).toBe(state.source.h);
      expect(state.layer).toMatchObject({ from: 0, w: state.w, h: state.h });
      if (file.endsWith('.mp4')) {
        expect(state.fps).toBe(30);
        expect(state.exportFps).toBe(30);
        expect(state.dur).toBe(state.source.dur);
        expect(state.layer.dur).toBe(state.source.dur);
        expect(state.work).toEqual([0, state.source.dur]);
      } else {
        expect(state.fps).toBe(60);
        expect(state.dur).toBe(10);
      }
    });
  }
});
