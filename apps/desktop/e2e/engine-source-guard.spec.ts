/** Independent of pixel assertions, so parity failures cannot hide legacy calls. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('native source consumers never invoke browser capture playback or export conversion', async ({ session }) => {
  test.setTimeout(180_000);
  const evidence = process.env.PM_SOURCE_EVIDENCE;
  const directory = evidence ? path.join(evidence, 'guard') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-guard-'));
  await mkdir(directory, { recursive: true });
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      (window as any).__sourceCalls = { captures: 0, plays: 0, conversions: 0 };
      const draw = CanvasRenderingContext2D.prototype.drawImage;
      CanvasRenderingContext2D.prototype.drawImage = function (...args: any[]) {
        if (args[0] instanceof HTMLVideoElement) {
          (window as any).__sourceCalls.captures++;
          throw new Error('Native source consumer captured a browser video');
        }
        return (draw as any).apply(this, args);
      };
    });
    for (const name of ['still-red.png', 'h264-aac.mp4', 'tone.wav', 'prores422.mov', 'still.heic']) {
      await importFixture(page, name);
      await page.waitForFunction(name => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === name), name);
    }
    await page.waitForFunction(() => !!(window as any).PM.EngineRender?.ready);
    await page.evaluate(() => {
      // Import's browser-codec probe is complete. Source viewing must now use
      // the engine voice, and export conversion must use a rendition job.
      HTMLMediaElement.prototype.play = function () {
        (window as any).__sourceCalls.plays++;
        return Promise.reject(new Error('Native source consumer played browser media'));
      };
      (window as any).PM.MediaStore.convert = async () => {
        (window as any).__sourceCalls.conversions++;
        throw new Error('Native code export invoked browser-host conversion');
      };
    });
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const ids = Object.fromEntries(Object.values<any>(PM.proj.assets).map(a => [a.name, a.id]));
      const source = PM.EngineSourceMedia;
      await source.poster(ids['h264-aac.mp4']);
      const strip = await source.filmstrip(ids['h264-aac.mp4'], [.1, .5], 160, 90);
      const viewer = PM.Kernel.services.get('viewer');
      viewer.preview.show(ids['h264-aac.mp4']);
      await new Promise<void>((resolve, reject) => {
        const monitor = source.monitorValue;
        const timer = setTimeout(() => reject(new Error('Guard source monitor did not paint')), 15_000);
        monitor.addEventListener('loadeddata', () => { clearTimeout(timer); resolve(); }, { once: true });
        monitor.currentTime = .25;
      });
      viewer.preview.clear();
      viewer.preview.toggle(ids['tone.wav']);
      const until = performance.now() + 15_000;
      while (!source.monitorValue.levels.peak.some((v: number) => v > .001)) {
        if (performance.now() > until) throw new Error('Guard source audio never produced levels');
        await new Promise(resolve => setTimeout(resolve, 20));
      }
      viewer.preview.clear();
      PM.proj.layers = [PM.mkLayer('video', { d: { asset: ids['prores422.mov'] } }, PM.proj),
        PM.mkLayer('image', { d: { asset: ids['still.heic'] } }, PM.proj)];
      PM.touch();
      const before = source.stats.renditions;
      const exported = await PM.Export.buildWeb();
      return { ...((window as any).__sourceCalls), jobs: source.stats.renditions - before,
        frames: strip.length, exportedFiles: exported.files.size, browserMedia: document.querySelectorAll('.asset-preview.video video, #source-preview video, #source-preview audio').length };
    });
    await writeFile(path.join(directory, 'legacy-invocations.json'), JSON.stringify(result, null, 2));
    expect(result).toMatchObject({ captures: 0, plays: 0, conversions: 0, jobs: 2, frames: 2, browserMedia: 0 });
    expect(result.exportedFiles).toBeGreaterThanOrEqual(2);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
});
