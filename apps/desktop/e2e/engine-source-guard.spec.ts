/** Independent of pixel assertions, so parity failures cannot hide legacy calls. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('native source consumers never invoke browser capture, URL playback, or export conversion', async ({ session }) => {
  test.setTimeout(180_000);
  const evidence = process.env.PM_SOURCE_EVIDENCE;
  const directory = evidence ? path.join(evidence, 'guard') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-guard-'));
  await mkdir(directory, { recursive: true });
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      (window as any).__sourceCalls = { captures: 0, plays: 0, nativePresenterPlays: 0, conversions: 0 };
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
      // Import's browser-codec probe is complete. Only the private engine
      // MediaStream presenter may play; URL-backed media still fails closed.
      const play = HTMLMediaElement.prototype.play;
      HTMLMediaElement.prototype.play = function () {
        if (this instanceof HTMLVideoElement && this.dataset.nativeSource === 'true'
            && !this.src && !this.currentSrc && this.srcObject instanceof MediaStream
            && this.srcObject.getVideoTracks().length === 1
            && this.srcObject.getVideoTracks()[0]!.readyState === 'live'
            && this.srcObject.getAudioTracks().length === 0 && this.muted) {
          (window as any).__sourceCalls.nativePresenterPlays++;
          return play.call(this);
        }
        (window as any).__sourceCalls.plays++;
        return Promise.reject(new Error('Native source consumer played URL-backed or unverified browser media'));
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
      const monitorShapes: Array<Record<string, unknown>> = [];
      const inspectMonitor = (stage: string) => {
        const elements = [...document.querySelectorAll<HTMLMediaElement>('#source-preview video, #source-preview audio')];
        const shape = { stage, elements: elements.map(element => ({ tag: element.tagName,
          nativeSource: element instanceof HTMLVideoElement && element.dataset.nativeSource === 'true',
          srcEmpty: !element.src && !element.currentSrc,
          privateStream: element.srcObject instanceof MediaStream,
          liveVideoTracks: element.srcObject instanceof MediaStream
            ? element.srcObject.getVideoTracks().filter(track => track.readyState === 'live').length : 0,
          audioTracks: element.srcObject instanceof MediaStream ? element.srcObject.getAudioTracks().length : 0,
          muted: element.muted })) };
        monitorShapes.push(shape);
        if (shape.elements.some(element => !element.srcEmpty)) throw new Error(`URL-backed source monitor media at ${stage}`);
        return shape;
      };
      viewer.preview.show(ids['h264-aac.mp4']);
      inspectMonitor('video opened');
      await new Promise<void>((resolve, reject) => {
        const monitor = source.monitorValue;
        const timer = setTimeout(() => reject(new Error('Guard source monitor did not paint')), 15_000);
        monitor.addEventListener('loadeddata', () => { clearTimeout(timer); resolve(); }, { once: true });
        monitor.currentTime = .25;
      });
      const videoShape = inspectMonitor('video painted');
      if (videoShape.elements.length !== 1 || videoShape.elements[0]?.tag !== 'VIDEO'
          || !videoShape.elements[0]?.nativeSource || !videoShape.elements[0]?.privateStream
          || videoShape.elements[0]?.liveVideoTracks !== 1 || videoShape.elements[0]?.audioTracks !== 0
          || !videoShape.elements[0]?.muted)
        throw new Error('Source monitor is not the private video-only engine MediaStream');
      viewer.preview.clear();
      viewer.preview.toggle(ids['tone.wav']);
      inspectMonitor('audio opened');
      const until = performance.now() + 15_000;
      while (!source.monitorValue.levels.peak.some((v: number) => v > .001)) {
        if (performance.now() > until) throw new Error('Guard source audio never produced levels');
        await new Promise(resolve => setTimeout(resolve, 20));
      }
      inspectMonitor('audio levels');
      viewer.preview.clear();
      PM.proj.layers = [PM.mkLayer('video', { d: { asset: ids['prores422.mov'] } }, PM.proj),
        PM.mkLayer('image', { d: { asset: ids['still.heic'] } }, PM.proj)];
      PM.touch();
      const before = source.stats.renditions;
      const exported = await PM.Export.buildWeb();
      return { ...((window as any).__sourceCalls), jobs: source.stats.renditions - before,
        presented: source.stats.presented, monitorShapes,
        frames: strip.length, exportedFiles: exported.files.size, browserMedia: document.querySelectorAll('.asset-preview.video video, #source-preview video, #source-preview audio').length };
    });
    await writeFile(path.join(directory, 'legacy-invocations.json'), JSON.stringify(result, null, 2));
    expect(result).toMatchObject({ captures: 0, plays: 0, conversions: 0, jobs: 2, frames: 2, browserMedia: 0 });
    expect(result.nativePresenterPlays).toBeGreaterThanOrEqual(1);
    expect(result.presented).toBeGreaterThanOrEqual(1);
    expect(result.exportedFiles).toBeGreaterThanOrEqual(2);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
});
