/** Source surfaces in hidden, isolated app profiles; no user media or screen captures. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';
import { capturedPixelDelta, decodedRenditionIdentity, renditionAudioFixture, sourcePcmComparison } from './helpers/source-media';

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
      await page.evaluate(enabled => {
        const state = (window as any).__sourceGuard = { enabled, references: false, videoDraws: 0, plays: 0 };
        const draw = CanvasRenderingContext2D.prototype.drawImage;
        CanvasRenderingContext2D.prototype.drawImage = function (...args: any[]) {
          if (args[0] instanceof HTMLVideoElement && !state.references) {
            state.videoDraws++;
            if (state.enabled) throw new Error('Native source surface invoked a browser video capture');
          }
          return (draw as any).apply(this, args);
        };
      }, enabled);
      for (const file of ['still-red.png', 'h264-aac.mp4', 'tone.wav', 'prores422.mov', 'still.heic']) {
        await importFixture(page, file);
        await page.waitForFunction(name => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === name), file);
      }
      const audioFile = await renditionAudioFixture(directory);
      const chooser = page.waitForEvent('filechooser');
      await page.evaluate(() => (window as any).PM.pickFiles());
      await (await chooser).setFiles(audioFile);
      await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === 'rendition-audio.mov'));
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
          const referenceDraw = () => {
            (window as any).__sourceGuard.references = true;
            try { context.drawImage(asset.el, 0, 0, canvas.width, canvas.height); }
            finally { (window as any).__sourceGuard.references = false; }
          };
          const began = performance.now();
          for (const time of times) {
            if (meta.kind === 'video') await seek(asset.el, time);
            context.clearRect(0, 0, width, height); referenceDraw();
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
          context.clearRect(0, 0, canvas.width, canvas.height); referenceDraw();
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
          // Source monitor fitting must preserve fractional edges and SVG text.
          if (name === 'source.svg') {
            frames[name].fits = [];
            canvas.width = 317; canvas.height = 181;
            for (const fit of ['contain', 'cover']) {
              const scale = (fit === 'contain' ? Math.min : Math.max)(canvas.width / meta.w, canvas.height / meta.h);
              const w = meta.w * scale, h = meta.h * scale;
              context.clearRect(0, 0, canvas.width, canvas.height);
              context.drawImage(asset.el, (canvas.width - w) / 2, (canvas.height - h) / 2, w, h);
              const expected = context.getImageData(0, 0, canvas.width, canvas.height).data;
              const reference = canvas.toDataURL('image/png');
              const fresh = document.createElement('canvas'); fresh.width = canvas.width; fresh.height = canvas.height;
              const freshContext = fresh.getContext('2d')!;
              freshContext.drawImage(asset.el, (fresh.width - w) / 2, (fresh.height - h) / 2, w, h);
              const freshExpected = freshContext.getImageData(0, 0, fresh.width, fresh.height).data;
              const freshReference = fresh.toDataURL('image/png');
              let max = 0, changed = 0, freshMax = 0, freshChanged = 0, native = null;
              if (enabled) {
                const received = (await source.frame(ids[name], 0, canvas.width, canvas.height, { fit })).data;
                for (let i = 0; i < expected.length; i++) {
                  const d = Math.abs(expected[i]! - received[i]!); max = Math.max(max, d); if (d) changed++;
                  const freshDelta = Math.abs(freshExpected[i]! - received[i]!); freshMax = Math.max(freshMax, freshDelta); if (freshDelta) freshChanged++;
                }
                context.putImageData(new ImageData(new Uint8ClampedArray(received), canvas.width, canvas.height), 0, 0);
                native = canvas.toDataURL('image/png');
              }
              // Keep reference rendering independent of the native PNG written
              // into the reused canvas above. Its readback/backend history is
              // diagnostic only; each source request draws into a fresh canvas.
              frames[name].fits.push({ fit, max: freshMax, changed: freshChanged,
                reusedMax: max, reusedChanged: changed, reference: freshReference, reusedReference: reference, native });
            }
          }
        }
        return frames;
      }, { enabled, ids });
      await writeFile(path.join(directory, 'measurements.json'), JSON.stringify(measured, null, 2));
      for (const [name, result] of Object.entries<any>(measured)) {
        for (const kind of ['reference', 'native', 'posterReference', 'posterNative']) if (result[kind]) await writeFile(path.join(directory, `${name}-${kind}.png`), Buffer.from(result[kind].split(',')[1], 'base64'));
        for (const fit of result.fits ?? []) for (const kind of ['reference', 'reusedReference', 'native']) if (fit[kind]) await writeFile(path.join(directory, `${name}-${fit.fit}-${kind}.png`), Buffer.from(fit[kind].split(',')[1], 'base64'));
      }
      const inline = [];
      for (const [name, result] of Object.entries<any>(measured)) {
        const poster = page.locator(`.asset-card[data-asset-id="${ids[name]}"] .asset-poster`);
        await expect(poster).toBeVisible();
        const decoded = await poster.evaluate(async (element: HTMLImageElement, referenceUrl: string) => {
          await element.decode();
          const referenceBytes = Uint8Array.from(atob(referenceUrl.split(',')[1]!), c => c.charCodeAt(0));
          const bitmap = await createImageBitmap(new Blob([referenceBytes], { type: 'image/png' }));
          try {
            const canvas = document.createElement('canvas'); canvas.width = bitmap.width; canvas.height = bitmap.height;
            const context = canvas.getContext('2d')!;
            context.drawImage(bitmap, 0, 0);
            const expected = context.getImageData(0, 0, canvas.width, canvas.height).data;
            context.clearRect(0, 0, canvas.width, canvas.height); context.drawImage(element, 0, 0);
            const actual = context.getImageData(0, 0, canvas.width, canvas.height).data;
            let max = 0, changed = 0;
            for (let i = 0; i < expected.length; i++) { const d = Math.abs(expected[i]! - actual[i]!); max = Math.max(max, d); if (d) changed++; }
            return { max, changed, width: element.naturalWidth, height: element.naturalHeight, png: canvas.toDataURL('image/png') };
          } finally { bitmap.close(); }
        }, result.posterReference);
        const { png, ...metric } = decoded;
        inline.push({ name, ...metric });
        await writeFile(path.join(directory, `${name}-inline.png`), Buffer.from(png.split(',')[1]!, 'base64'));
      }
      await writeFile(path.join(directory, 'inline-thumbnails.json'), JSON.stringify(inline, null, 2));

      // The offline probe uses the production SourceAudio document and mixer.
      // Chromium's decoded PCM is padded to the media element's duration, just
      // as the source monitor hears silence after the final decoded sample.
      const pcmReferences = await page.evaluate(async ids => {
        const PM = (window as any).PM, samples = [];
        for (const [name, file] of [['tone.wav', 'source-tone.wav'], ['rendition-audio.mov', 'source-proxy.webm']]) {
          const live = PM.assets.get(ids[name!]);
          const blob = live.audioBlob instanceof Blob ? live.audioBlob : await (await fetch(live.url)).blob();
          const bytes = await blob.arrayBuffer();
          const context = new AudioContext({ sampleRate: 48000 });
          try {
            const decoded = await context.decodeAudioData(bytes.slice(0));
            let duration = Number(live.el?.duration);
            if (!(duration > 0 && Number.isFinite(duration))) duration = decoded.duration;
            const frames = Math.round(duration * 48000), stereo = new Float32Array(frames * 2);
            for (let channel = 0; channel < 2; channel++) {
              const input = decoded.getChannelData(Math.min(channel, decoded.numberOfChannels - 1));
              for (let frame = 0; frame < Math.min(frames, input.length); frame++) stereo[frame * 2 + channel] = input[frame]!;
            }
            samples.push({ file: file!, input: [...new Uint8Array(bytes)], expected: [...stereo], decodedFrames: decoded.length, mediaFrames: frames });
          } finally { await context.close(); }
        }
        return samples;
      }, ids);
      const pcm = [];
      for (const sample of pcmReferences) pcm.push({ name: sample.file, decodedFrames: sample.decodedFrames, mediaFrames: sample.mediaFrames,
        ...await sourcePcmComparison(directory, sample.file, sample.input, sample.expected) });
      await writeFile(path.join(directory, 'source-audio-pcm.json'), JSON.stringify(pcm, null, 2));
      for (const result of pcm) {
        expect.soft(result.sameFrames, JSON.stringify(result)).toBe(true);
        expect.soft(result.signalRms).toBeGreaterThan(.001);
        expect.soft(result.rmsResidualDbfs, JSON.stringify(result)).toBeLessThanOrEqual(-85);
      }

      // Guard the consumers after taking references: source viewing must not play a browser element.
      await page.evaluate(() => {
        (window as any).__sourceBrowserPlays = 0;
        const play = HTMLMediaElement.prototype.play;
        HTMLMediaElement.prototype.play = function () { (window as any).__sourceBrowserPlays++; return play.call(this); };
      });
      await expect(page.locator('.toast')).toHaveCount(0, { timeout: 15_000 });
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
        for (const name of Object.keys(measured)) await page.locator(`.asset-card[data-asset-id="${ids[name]}"] .asset-preview`).screenshot({ path: path.join(directory, `${name}-inline-${theme}.png`), animations: 'disabled' });
      }
      if (enabled && evidence) {
        const captures = [];
        for (const theme of ['light', 'dark']) for (const surface of ['source', ...Object.keys(measured).map(name => `${name}-inline`)]) {
          const file = `${surface}-${theme}.png`;
          captures.push({ file, ...await capturedPixelDelta(path.join(evidence, 'legacy', file), path.join(directory, file)) });
        }
        await writeFile(path.join(evidence, 'theme-capture-comparison.json'), JSON.stringify(captures, null, 2));
        for (const result of captures) {
          expect.soft(result.sameSize, JSON.stringify(result)).toBe(true);
          expect.soft(result.max, JSON.stringify(result)).toBe(0);
        }
      }
      const monitor = await page.evaluate(async ({ enabled, id }) => {
        const PM = (window as any).PM; PM.Kernel.services.get('viewer').preview.show(id);
        const root = document.querySelector<HTMLElement>('#source-preview')!;
        root.style.width = `${320 / devicePixelRatio}px`; root.style.height = `${180 / devicePixelRatio}px`;
        root.style.right = root.style.bottom = 'auto';
        const source = enabled ? PM.EngineSourceMedia.monitorValue : root.querySelector('video');
        const painted = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        const until = performance.now() + 15_000;
        while (enabled ? source.canvas.width !== 320 || source.canvas.height !== 180 : source.readyState < 2) {
          if (performance.now() > until) throw new Error('Source monitor did not reach its comparison size');
          await painted();
        }
        const samples: number[] = [];
        const pixels: Array<{ time: number; max: number; changed: number; reference: string; native: string | null }> = [];
        const reference = document.createElement('canvas'); reference.width = 320; reference.height = 180;
        const context = reference.getContext('2d')!;
        const video: HTMLVideoElement = PM.assets.get(id).el;
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
          if (Math.abs(video.currentTime - time) > .000001 || video.readyState < 2) await new Promise<void>((resolve, reject) => {
            const timer = setTimeout(() => { video.removeEventListener('seeked', done); reject(new Error('Monitor reference seek timed out')); }, 10_000);
            const done = () => { clearTimeout(timer); video.removeEventListener('seeked', done); resolve(); };
            video.addEventListener('seeked', done); video.currentTime = time;
          });
          const scale = Math.min(320 / video.videoWidth, 180 / video.videoHeight);
          const width = video.videoWidth * scale, height = video.videoHeight * scale;
          context.clearRect(0, 0, 320, 180);
          (window as any).__sourceGuard.references = true;
          try { context.drawImage(video, (320 - width) / 2, (180 - height) / 2, width, height); }
          finally { (window as any).__sourceGuard.references = false; }
          const expected = context.getImageData(0, 0, 320, 180).data;
          const actual = enabled ? source.canvas.getContext('2d').getImageData(0, 0, 320, 180).data : expected;
          let max = 0, changed = 0;
          for (let i = 0; i < expected.length; i++) { const d = Math.abs(expected[i]! - actual[i]!); max = Math.max(max, d); if (d) changed++; }
          pixels.push({ time, max, changed, reference: reference.toDataURL('image/png'), native: enabled ? source.canvas.toDataURL('image/png') : null });
        }
        PM.Kernel.services.get('viewer').preview.clear();
        return { samples, pixels };
      }, { enabled, id: ids['h264-aac.mp4'] });
      await writeFile(path.join(directory, 'source-monitor-scrub.json'), JSON.stringify(monitor));
      for (const [index, frame] of monitor.pixels.entries()) {
        for (const kind of ['reference', 'native'] as const) if (frame[kind]) await writeFile(path.join(directory, `monitor-${index}-${kind}.png`), Buffer.from(frame[kind]!.split(',')[1]!, 'base64'));
      }
      const lifecycle = [];
      for (const name of ['h264-aac.mp4', 'tone.wav', 'prores422.mov', 'rendition-audio.mov']) {
        await page.evaluate(({ id, enabled }) => {
          const PM = (window as any).PM, preview = PM.Kernel.services.get('viewer').preview;
          preview.show(id);
          const source = enabled ? PM.EngineSourceMedia.monitorValue : document.querySelector('#source-preview video, #source-preview audio');
          (window as any).__sourceEnd = false;
          source.addEventListener('ended', () => { (window as any).__sourceEnd = true; }, { once: true });
          preview.toggle(id);
        }, { id: ids[name], enabled });
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
        lifecycle.push(await page.evaluate(({ name, enabled }) => {
          const source = enabled ? (window as any).PM.EngineSourceMedia.monitorValue : document.querySelector('#source-preview video, #source-preview audio');
          return { name, ended: (window as any).__sourceEnd, time: source.currentTime, duration: source.duration };
        }, { name, enabled }));
        await page.locator('#source-preview .sp-scrub').focus(); await page.keyboard.press('Home');
        await page.keyboard.press('Escape');
      }
      await writeFile(path.join(directory, 'source-monitor-lifecycle.json'), JSON.stringify(lifecycle, null, 2));
      for (const result of lifecycle) expect.soft(result.ended, JSON.stringify(result)).toBe(true);
      expect(await page.evaluate(() => (window as any).__sourceBrowserPlays)).toBe(enabled ? 0 : 4);

      const code = await page.evaluate(async ({ enabled, id, still }) => {
        const PM = (window as any).PM;
        PM.Kernel.services.get('viewer').preview.clear();
        PM.proj.layers = [PM.mkLayer('video', { d: { asset: id } }, PM.proj), PM.mkLayer('image', { d: { asset: still } }, PM.proj)]; PM.touch();
        const references: Record<string, number[]> = {};
        for (const key of [id, still]) {
          const meta = PM.proj.assets[key];
          const hash = meta.contentHash ?? meta.link?.hash;
          const kept = meta.rendition && hash ? await PM.MediaStore.rendition.get(hash) : null;
          const blob = kept ?? await (await fetch(PM.assets.get(key).url)).blob();
          references[key] = [...new Uint8Array(await blob.arrayBuffer())];
        }
        const before = PM.EngineSourceMedia?.stats.renditions ?? 0;
        const exported = await PM.Export.buildWeb();
        return { media: exported.scene.assets[id], still: exported.scene.assets[still],
          videoReference: references[id], imageReference: references[still],
          videoActual: [...exported.files.get(exported.scene.assets[id])], imageActual: [...exported.files.get(exported.scene.assets[still])],
          videoDraws: (window as any).__sourceGuard.videoDraws,
          jobs: (PM.EngineSourceMedia?.stats.renditions ?? 0) - before, enabled };
      }, { enabled, id: ids['rendition-audio.mov'], still: ids['still.heic'] });
      expect(code.media).toMatch(/\.webm$/);
      expect(code.still).toMatch(/\.png$/);
      if (enabled) expect(code.jobs).toBe(2);
      const identity = {
        video: await decodedRenditionIdentity(directory, 'export-video', 'video', code.videoReference!, code.videoActual),
        image: await decodedRenditionIdentity(directory, 'export-image', 'image', code.imageReference!, code.imageActual)
      };
      await writeFile(path.join(directory, 'rendition-identity.json'), JSON.stringify(identity, null, 2));
      expect(identity.video.picture.equal).toBe(true); expect(identity.video.audio?.equal).toBe(true);
      expect(identity.image.picture.equal).toBe(true);
      if (enabled) expect(code.videoDraws).toBe(0);
      expect(session.diagnostics.pageErrors).toEqual([]);
      if (enabled) {
        for (const result of inline) expect.soft(result.max, `inline ${result.name}: ${JSON.stringify(result)}; ${directory}`).toBe(0);
        for (const frame of monitor.pixels) expect(frame.max, `source monitor at ${frame.time}: ${JSON.stringify({ max: frame.max, changed: frame.changed })}; ${directory}`).toBe(0);
        for (const [name, result] of Object.entries<any>(measured)) {
          for (const fit of result.fits ?? []) expect.soft(fit.max, `${name} ${fit.fit}: ${JSON.stringify({ max: fit.max, changed: fit.changed })}; ${directory}`).toBe(0);
          for (const metric of result.delta) expect(metric.max, `${name}: ${JSON.stringify(metric)}; ${directory}`).toBe(0);
          expect(result.posterDelta.max, `${name} poster: ${JSON.stringify(result.posterDelta)}; ${directory}`).toBe(0);
        }
        expect(measured['h264-aac.mp4'].filmstripEqual).toBe(true);
        expect(measured['source.svg'].filmstripEqual).toBe(true);
      }
    } finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
  });
});
