/** Source surfaces in hidden, isolated app profiles; no user media or screen captures. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

for (const enabled of [false, true]) test.describe(`source media, engine ${enabled ? 'on' : 'off'}`, () => {
  test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } } });

  test('posters, source frames, audio and code renditions retain their shipped behavior', async ({ session }) => {
    test.setTimeout(240_000);
    const evidence = process.env.PM_SOURCE_EVIDENCE;
    const directory = evidence ? path.join(evidence, enabled ? 'native' : 'legacy') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-surfaces-'));
    await mkdir(directory, { recursive: true });
    try {
      await session.openEditor();
      const { page } = session;
      for (const file of ['still-red.png', 'h264-aac.mp4', 'tone.wav', 'prores422.mov', 'still.heic']) {
        await importFixture(page, file);
        await page.waitForFunction(name => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === name), file);
      }
      await page.evaluate(async () => {
        const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><style>text{font:31px system-ui;fill:rebeccapurple}</style><rect x="7.25" y="11.5" width="91.5" height="79.25" rx="13.5" fill="#f57c14"/><text x="18" y="145">Source &amp; type</text></svg>';
        await (window as any).PM.importFiles([new File([svg], 'source.svg', { type: 'image/svg+xml' })], { placement: null });
      });
      const ids = await page.evaluate(() => Object.fromEntries(Object.values((window as any).PM.proj.assets).map((a: any) => [a.name, a.id])));
      if (enabled) await page.waitForFunction(() => !!(window as any).PM.EngineRender?.ready);
      for (const name of ['still-red.png', 'h264-aac.mp4']) {
        await expect(page.locator('.asset-card').filter({ hasText: name }).locator('.asset-poster')).toBeVisible();
      }
      if (enabled) await expect(page.locator('.asset-preview.video video')).toHaveCount(0);

      const measured = await page.evaluate(async ({ enabled, ids }) => {
        const PM = (window as any).PM;
        const source = PM.EngineSourceMedia;
        const frames: Record<string, any> = {};
        const seek = async (video: HTMLVideoElement, time: number) => {
          if (Math.abs(video.currentTime - time) < .000001 && video.readyState >= 2) return;
          await new Promise<void>((resolve, reject) => {
            const timer = setTimeout(() => { video.removeEventListener('seeked', done); reject(new Error('Source seek timed out')); }, 10_000);
            const done = () => { clearTimeout(timer); video.removeEventListener('seeked', done); resolve(); };
            video.addEventListener('seeked', done); video.currentTime = time;
          });
        };
        for (const name of ['still-red.png', 'h264-aac.mp4', 'prores422.mov', 'source.svg', 'still.heic']) {
          const asset = PM.assets.get(ids[name]), meta = PM.proj.assets[ids[name]];
          const scale = Math.min(1, 320 / Math.max(meta.w, meta.h));
          const width = Math.round(meta.w * scale), height = Math.round(meta.h * scale);
          const times = meta.kind === 'image' ? [0] : [0, .25, .5];
          const references: Uint8Array[] = [];
          const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
          const context = canvas.getContext('2d')!;
          const began = performance.now();
          for (const time of times) {
            if (meta.kind === 'video') await seek(asset.el, time);
            context.clearRect(0, 0, width, height); context.drawImage(asset.el, 0, 0, width, height);
            references.push(new Uint8Array(context.getImageData(0, 0, width, height).data));
          }
          const legacyMs = performance.now() - began;
          let nativeMs = 0;
          const actual: Uint8Array[] = [];
          if (enabled) {
            const began = performance.now();
            for (const time of times) actual.push(new Uint8Array((await source.frame(ids[name], time, width, height)).data));
            nativeMs = performance.now() - began;
          }
          const delta = actual.map((frame, n) => {
            let changed = 0, max = 0;
            for (let i = 0; i < frame.length; i++) { const d = Math.abs(frame[i]! - references[n]![i]!); if (d) changed++; max = Math.max(max, d); }
            return { changed, max };
          });
          const image = (bytes: Uint8Array) => { context.putImageData(new ImageData(new Uint8ClampedArray(bytes), width, height), 0, 0); return canvas.toDataURL('image/png'); };
          const index = Math.max(0, delta.findIndex(d => d.max > 0));
          frames[name] = { delta, legacyMs, nativeMs, capturedTime: times[index], reference: image(references[index]!), native: enabled ? image(actual[index]!) : null };
          // Match the shipped poster's upscale, timestamp, and lossy encoder too.
          const posterScale = 320 / Math.max(meta.w, meta.h);
          canvas.width = Math.round(meta.w * posterScale); canvas.height = Math.round(meta.h * posterScale);
          if (meta.kind === 'video') await seek(asset.el, Math.min(.5, meta.dur * .1));
          context.clearRect(0, 0, canvas.width, canvas.height); context.drawImage(asset.el, 0, 0, canvas.width, canvas.height);
          const legacyPoster = await new Promise<Blob>(resolve => canvas.toBlob(blob => resolve(blob!), 'image/webp', .8));
          const decodePoster = async (blob: Blob) => {
            const bitmap = await createImageBitmap(blob);
            try { context.clearRect(0, 0, canvas.width, canvas.height); context.drawImage(bitmap, 0, 0); }
            finally { bitmap.close(); }
            return { data: new Uint8Array(context.getImageData(0, 0, canvas.width, canvas.height).data), png: canvas.toDataURL('image/png') };
          };
          const posterReference = await decodePoster(legacyPoster);
          frames[name].posterReference = posterReference.png;
          if (enabled) {
            const posterNative = await decodePoster(await source.poster(ids[name]));
            frames[name].posterNative = posterNative.png;
            let max = 0, changed = 0;
            posterNative.data.forEach((v, i) => { const d = Math.abs(v - posterReference.data[i]!); max = Math.max(max, d); if (d) changed++; });
            frames[name].posterDelta = { max, changed };
          }
          if (enabled && (name === 'h264-aac.mp4' || name === 'source.svg')) {
            const began = performance.now();
            const strip = await source.filmstrip(ids[name], times, width, height);
            frames[name].filmstripMs = performance.now() - began;
            frames[name].filmstripEqual = strip.every((frame: Uint8Array, n: number) => frame.every((v, i) => v === actual[n]![i]));
          }
        }
        return frames;
      }, { enabled, ids });
      await writeFile(path.join(directory, 'measurements.json'), JSON.stringify(measured, null, 2));
      for (const [name, result] of Object.entries<any>(measured)) {
        for (const kind of ['reference', 'native', 'posterReference', 'posterNative']) if (result[kind]) await writeFile(path.join(directory, `${name}-${kind}.png`), Buffer.from(result[kind].split(',')[1], 'base64'));
      }

      // Guard the consumers after taking references: source viewing must not play a browser element.
      await page.evaluate(() => {
        (window as any).__sourceBrowserPlays = 0;
        const play = HTMLMediaElement.prototype.play;
        HTMLMediaElement.prototype.play = function () { (window as any).__sourceBrowserPlays++; return play.call(this); };
      });
      for (const theme of ['light', 'dark']) {
        await page.evaluate(({ theme, id }) => {
          const PM = (window as any).PM; PM.theme.apply(theme);
          PM.Kernel.services.get('viewer').preview.show(id);
        }, { theme, id: ids['still-red.png'] });
        await expect(page.locator('#source-preview[data-open="true"]')).toBeVisible();
        if (enabled) await expect.poll(() => page.locator('#source-preview canvas').evaluate((c: HTMLCanvasElement) => c.width)).toBeGreaterThan(1);
        await page.locator('#source-preview').screenshot({ path: path.join(directory, `source-${theme}.png`), animations: 'disabled' });
        await page.keyboard.press('Escape');
        await page.locator('.asset-list').screenshot({ path: path.join(directory, `assets-${theme}.png`), animations: 'disabled' });
      }
      const scrubMs = await page.evaluate(async ({ enabled, id }) => {
        const PM = (window as any).PM; PM.Kernel.services.get('viewer').preview.show(id);
        const source = enabled ? PM.EngineSourceMedia.monitorValue : document.querySelector('#source-preview video');
        const samples: number[] = [];
        for (const time of [.25, .5, 0, .25]) {
          const started = performance.now();
          await new Promise<void>((resolve, reject) => {
            const event = enabled ? 'loadeddata' : 'seeked';
            const done = () => {
              if (Math.abs(source.currentTime - time) > .001) return;
              clearTimeout(timer); source.removeEventListener(event, done); resolve();
            };
            const timer = setTimeout(() => { source.removeEventListener(event, done); reject(new Error('Source monitor scrub timed out')); }, 10_000);
            source.addEventListener(event, done); source.currentTime = time;
          });
          samples.push(performance.now() - started);
        }
        PM.Kernel.services.get('viewer').preview.clear();
        return samples;
      }, { enabled, id: ids['h264-aac.mp4'] });
      await writeFile(path.join(directory, 'source-monitor-scrub.json'), JSON.stringify(scrubMs));
      for (const name of ['h264-aac.mp4', 'tone.wav', 'prores422.mov']) {
        await page.evaluate(id => (window as any).PM.Kernel.services.get('viewer').preview.toggle(id), ids[name]);
        await expect.poll(() => page.locator('#source-preview .sp-scrub').getAttribute('aria-valuenow')).not.toBe('0');
        if (enabled) {
          await expect(page.locator('#source-preview video, #source-preview audio')).toHaveCount(0);
          if (name !== 'prores422.mov') await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSourceMedia.monitorValue.levels.peak.some((p: number) => p > .001))).toBe(true);
        }
        await page.evaluate(() => {
          const PM = (window as any).PM;
          const source = PM.EngineSourceMedia?.monitorValue ?? document.querySelector('#source-preview video, #source-preview audio');
          source.currentTime = Math.max(0, source.duration - .05);
        });
        await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').preview.playing)).toBe(false);
        await page.locator('#source-preview .sp-scrub').focus(); await page.keyboard.press('Home');
        await page.keyboard.press('Escape');
      }
      expect(await page.evaluate(() => (window as any).__sourceBrowserPlays)).toBe(enabled ? 0 : 3);

      const code = await page.evaluate(async ({ enabled, id, still }) => {
        const PM = (window as any).PM;
        PM.Kernel.services.get('viewer').preview.clear();
        PM.proj.layers = [PM.mkLayer('video', { d: { asset: id } }, PM.proj), PM.mkLayer('image', { d: { asset: still } }, PM.proj)]; PM.touch();
        const before = PM.EngineSourceMedia?.stats.renditions ?? 0;
        const exported = await PM.Export.buildWeb();
        return { media: exported.scene.assets[id], still: exported.scene.assets[still], jobs: (PM.EngineSourceMedia?.stats.renditions ?? 0) - before, enabled };
      }, { enabled, id: ids['prores422.mov'], still: ids['still.heic'] });
      expect(code.media).toMatch(/\.webm$/);
      expect(code.still).toMatch(/\.png$/);
      if (enabled) expect(code.jobs).toBe(2);
      expect(session.diagnostics.pageErrors).toEqual([]);
      if (enabled) {
        for (const [name, result] of Object.entries<any>(measured)) {
          for (const metric of result.delta) expect(metric.max, `${name}: ${JSON.stringify(metric)}; ${directory}`).toBe(0);
          expect(result.posterDelta.max, `${name} poster: ${JSON.stringify(result.posterDelta)}; ${directory}`).toBe(0);
        }
        expect(measured['h264-aac.mp4'].filmstripEqual).toBe(true);
        expect(measured['source.svg'].filmstripEqual).toBe(true);
      }
    } finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
  });
});
