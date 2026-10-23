/** Capture the shipped Chromium decoder and texImage2D paths in a hidden app.
 * Build the app first. Generate outside the Electron lease, then capture under
 * pm-electron-lock. Operational evidence lives outside the repository; pack
 * copies only public synthetic media and pixel references into the fixture.
 *   node scripts/video-parity.mjs generate /absolute/evidence [--round2]
 *   pm-electron-lock node scripts/video-parity.mjs capture /absolute/evidence
 */
import { mkdir, readFile, writeFile, mkdtemp, rm, access, copyFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { deflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(desktop, '../..');
const [mode, destination, selection] = process.argv.slice(2);
if (!['generate', 'capture', 'png', 'pack'].includes(mode) || !destination || !path.isAbsolute(destination)
    || path.resolve(destination).startsWith(root + path.sep) || (selection && selection !== '--round2')) {
  throw new Error('usage: video-parity.mjs generate|capture|png|pack ABSOLUTE_DIRECTORY_OUTSIDE_REPO [--round2]');
}
await mkdir(destination, { recursive: true });
const recipes = [
  ['h264-601-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-709-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-709-matrix-only', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_range', 'tv']],
  ['h264-hd-709-matrix-only', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_range', 'tv']],
  ['h264-709-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['h264-2020-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-601-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['h264-2020-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['hevc10-601-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-709-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-2020-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-601-full', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error:range=full', '-tag:v', 'hvc1'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['hevc10-709-full', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error:range=full', '-tag:v', 'hvc1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['hevc10-2020-full', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error:range=full', '-tag:v', 'hvc1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['vp9-709-limited', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'realtime', '-cpu-used', '8'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['vp9-untagged', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'realtime', '-cpu-used', '8'], []],
  ['av1-709-limited', 'mp4', ['libsvtav1', '-pix_fmt', 'yuv420p', '-preset', '12', '-svtav1-params', 'lp=1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['prores422-709', 'mov', ['prores_ks', '-profile:v', '2', '-pix_fmt', 'yuv422p10le'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['prores4444-709', 'mov', ['prores_ks', '-profile:v', '4', '-pix_fmt', 'yuva444p10le'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
];
if (mode === 'pack') {
  const fixture = path.join(root, 'engine/crates/pm-render/tests/fixtures/video-parity-round2');
  const sources = JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
  const capture = JSON.parse(await readFile(path.join(destination, 'chromium.json'), 'utf8'));
  const references = [];
  // Validate the complete set before writing anything into the public repo.
  for (const source of sources) {
    const row = capture.rows.find(candidate => candidate.id === source.id);
    if (!row || row.error || row.frames?.length !== source.times.length) throw new Error(`Incomplete capture: ${source.id}: ${row?.error || 'missing frames'}`);
    const original = await readFile(path.join(destination, source.file));
    if (createHash('sha256').update(original).digest('hex') !== source.sourceSha256) throw new Error(`Source changed after generation: ${source.id}`);
    const mediaFile = row.playback?.file || source.file;
    if (source.id.startsWith('prores') && (!row.playback || !row.playback.playbackProxy || !row.playback.liveElementMatched)) throw new Error(`ProRes import bypassed playback proxy: ${source.id}`);
    if (row.playback) {
      const media = await readFile(path.join(destination, mediaFile));
      if (createHash('sha256').update(media).digest('hex') !== row.playback.sha256) throw new Error(`Playback rendition changed: ${source.id}`);
    }
    for (const [sample, frame] of row.frames.entries()) {
      if (frame.time !== source.times[sample] || frame.w !== source.probe.width || frame.h !== source.probe.height) throw new Error(`Bad frame metadata: ${source.id}-${sample}`);
      const expectedTime = Math.floor(frame.time * 12) / 12;
      if (Math.abs(frame.presentedMediaTime - expectedTime) > 0.02 || Math.abs(frame.timestamp / 1_000_000 - expectedTime) > 0.02) {
        throw new Error(`Stale or wrong presented frame: ${source.id}-${sample}`);
      }
      const refs = {};
      const pixels = {};
      if (frame.nativeReference !== `${source.id}-${sample}.premult`) throw new Error(`Missing native upload reference: ${source.id}-${sample}`);
      for (const kind of ['rgba', 'canvas', 'premult']) {
        const rgba = await readFile(path.join(destination, `${source.id}-${sample}.${kind}`));
        if (rgba.length !== frame.w * frame.h * 4) throw new Error(`Bad RGBA size: ${source.id}-${sample}.${kind}`);
        pixels[kind] = rgba;
        refs[kind] = { file: `${source.id}-${sample}.${kind}.rgba.zlib`, sha256: createHash('sha256').update(rgba).digest('hex') };
      }
      if (source.id === 'prores4444-709') {
        if (pixels.canvas.equals(pixels.premult)) throw new Error(`Alpha upload was not premultiplied: ${source.id}-${sample}`);
        for (let alpha = 3; alpha < pixels.canvas.length; alpha += 4) {
          if (pixels.canvas[alpha] !== pixels.premult[alpha]) throw new Error(`Alpha changed during upload: ${source.id}-${sample}`);
        }
      } else if (!pixels.canvas.equals(pixels.premult)) {
        throw new Error(`Opaque canvas changed during upload: ${source.id}-${sample}`);
      }
      references.push({ source: source.id, sample, time: frame.time, width: frame.w, height: frame.h,
        original: source.file, media: mediaFile, importPath: row.playback ? 'PM.pickFiles → VP9 playback proxy' : 'native Chromium video',
        format: frame.format, timestamp: frame.timestamp, colorSpace: frame.colorSpace,
        nativeReference: frame.nativeReference, actualCanvas: refs.canvas,
        premultipliedCanvas: refs.premult, texture: refs.rgba, rowOrder: 'sourceTopDown' });
    }
  }
  await mkdir(fixture, { recursive: true });
  for (const source of sources) {
    const row = capture.rows.find(candidate => candidate.id === source.id);
    await copyFile(path.join(destination, source.file), path.join(fixture, source.file));
    if (row.playback) await copyFile(path.join(destination, row.playback.file), path.join(fixture, row.playback.file));
    for (let sample = 0; sample < source.times.length; sample++) for (const kind of ['rgba', 'canvas', 'premult']) {
      const rgba = await readFile(path.join(destination, `${source.id}-${sample}.${kind}`));
      await writeFile(path.join(fixture, `${source.id}-${sample}.${kind}.rgba.zlib`), deflateSync(rgba, { level: 9 }));
    }
  }
  await writeFile(path.join(fixture, 'references.json'), JSON.stringify({
    capturedWith: capture.versions, sources: sources.map(({ id, file, times, probe, sourceSha256 }) => ({
      id, file, times, sourceSha256, codec: probe.codec_name, pixelFormat: probe.pix_fmt,
      colorRange: probe.color_range, colorSpace: probe.color_space, colorPrimaries: probe.color_primaries,
      colorTransfer: probe.color_transfer,
      playback: capture.rows.find(candidate => candidate.id === id).playback || null,
    })), references,
  }, null, 2));
} else if (mode === 'generate') {
  const rows = [];
  for (const [id, ext, codec, tags] of recipes.filter(([id]) => !selection
    || id.startsWith('hevc10-') || id.startsWith('prores'))) {
    const file = `${id}.${ext}`;
    const alpha = selection === '--round2' && id === 'prores4444-709';
    const colour = id.includes('601') ? 6 : id.includes('2020') ? 9 : 1;
    const bitstream = id.startsWith('h264-') && !id.endsWith('matrix-only')
      ? ['-bsf:v', `h264_metadata=colour_primaries=${colour}:transfer_characteristics=1:matrix_coefficients=${colour}:video_full_range_flag=${id.endsWith('full') ? 1 : 0}`]
      : id.startsWith('hevc10-')
        ? ['-bsf:v', `hevc_metadata=colour_primaries=${colour}:transfer_characteristics=1:matrix_coefficients=${colour}:video_full_range_flag=${id.endsWith('full') ? 1 : 0}`]
        : [];
    // Detail, gradients, motion and midtones in every codec; deliberately not
    // only saturated patches, which hide reconstruction and rounding errors.
    execFileSync(process.env.PM_PARITY_FFMPEG || 'ffmpeg', ['-v', 'error', '-y', '-f', 'lavfi', '-i',
      `testsrc2=size=${id.includes('-hd-') ? '1280x720' : '256x144'}:rate=12:duration=1.5`, '-vf',
      `${alpha ? 'format=yuva444p10le,' : ''}geq=lum='32+0.65*lum(X,Y)+20*X/W':cb='128+0.65*(cb(X,Y)-128)':cr='128+0.65*(cr(X,Y)-128)'${alpha ? ":a='256+767*X/W'" : ''}`,
      '-filter_threads', '1', '-threads', '1', '-c:v', ...codec, ...tags, ...bitstream, '-an', path.join(destination, file)],
    { stdio: 'inherit' });
    const probe = JSON.parse(execFileSync(process.env.PM_PARITY_FFPROBE || 'ffprobe', ['-v', 'error', '-show_streams', '-of', 'json', path.join(destination, file)], { encoding: 'utf8' }));
    rows.push({ id, file, times: [0.125, 0.625, 1.125], probe: probe.streams[0],
      sourceSha256: createHash('sha256').update(await readFile(path.join(destination, file))).digest('hex') });
  }
  await writeFile(path.join(destination, 'sources.json'), JSON.stringify(rows, null, 2));
} else {
  await access(path.join(desktop, 'out/main/index.js'));
  const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-video-parity-'));
  let app;
  try {
    const env = { ...process.env, POWERMOVE_USER_DATA: userData, POWERMOVE_BACKGROUND_TEST: '1',
      POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '0', POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
    delete env.ELECTRON_RUN_AS_NODE;
    app = await _electron.launch({ args: [path.join(desktop, 'out/main/index.js')], cwd: desktop, env, timeout: 120_000 });
    const page = await app.firstWindow();
    await page.waitForFunction(() => Boolean(window.PM?.mkProject && window.PM?.GL), undefined, { timeout: 60_000 });
    await page.evaluate(() => {
      const PM = window.PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', {
        detail: PM.mkProject({ w: 256, h: 144, fps: 12, dur: 3, name: 'Synthetic video parity' }),
      }));
      PM.ProjectsScreen.hide();
      PM.setTime(0, { raw: true, force: true });
    });
    const rows = mode === 'png' ? [] : JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
    const report = { versions: await app.evaluate(() => process.versions), rows: [] };
    for (const row of rows) {
      const imported = row.id.startsWith('prores');
      if (imported) {
        // The shipped app imports unsupported ProRes through its real picker
        // and converts it to the VP9 playback rendition used by the timeline.
        const chooser = page.waitForEvent('filechooser');
        await page.evaluate(() => window.PM.pickFiles());
        await (await chooser).setFiles(path.join(destination, row.file));
        await page.waitForFunction(name => [...window.PM.assets.map.values()].some(asset => asset.name === name),
          row.file, { timeout: 120_000 });
      }
      const bytes = imported ? null : [...await readFile(path.join(destination, row.file))];
      const captured = await page.evaluate(async ({ bytes, row, imported }) => {
        const encode = bytes => {
          let binary = '';
          for (let offset = 0; offset < bytes.length; offset += 8192) binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
          return btoa(binary);
        };
        const asset = imported ? [...window.PM.assets.map.values()].find(asset => asset.name === row.file) : null;
        if (imported && (!asset?.el?.src || !asset.playbackProxy)) throw new Error(`Imported asset ${row.file} has no playback video element`);
        const video = document.createElement('video');
        video.muted = true; video.preload = 'auto';
        let playback = null;
        let playbackBlob = null;
        if (imported) {
          playbackBlob = await window.PM.MediaStore.get(asset.storageKey);
          if (!playbackBlob || playbackBlob.type !== 'video/webm' || !asset.playbackProxy) {
            throw new Error(`Imported ${row.file} has no durable WebM playback proxy`);
          }
          const durable = new Uint8Array(await playbackBlob.arrayBuffer());
          const liveResponse = await fetch(asset.el.src);
          if (!liveResponse.ok) throw new Error(`Could not read live ${row.file} playback blob`);
          const live = new Uint8Array(await liveResponse.arrayBuffer());
          if (durable.length !== live.length || durable.some((byte, index) => byte !== live[index])) {
            throw new Error(`Imported ${row.file} persisted a different proxy than the live playback element`);
          }
          playback = { file: `${row.id}.playback.webm`, type: playbackBlob.type, size: playbackBlob.size,
            bytes: encode(durable), liveElementMatched: true,
            playbackProxy: asset.playbackProxy, playbackProxyVersion: asset.playbackProxyVersion };
        }
        const url = URL.createObjectURL(playbackBlob || new Blob([new Uint8Array(bytes)]));
        const timeout = (promise) => {
          let timer;
          return Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('video timeout')), 15_000); })])
            .finally(() => clearTimeout(timer));
        };
        try {
          await timeout(new Promise((resolve, reject) => {
            if (video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) return resolve();
            video.addEventListener('loadeddata', resolve, { once: true });
            video.addEventListener('error', () => reject(new Error(`HTMLVideoElement error ${video.error?.code}: ${video.error?.message}`)), { once: true });
            video.src = url;
          }));
          const frames = [];
          for (const time of row.times) {
            const seekStart = performance.now();
            const expectedFrameTime = Math.floor(time * 12) / 12;
            const presentation = new Promise((resolve, reject) => {
              const presented = (_now, metadata) => {
                if (metadata.mediaTime > expectedFrameTime + 0.02) return reject(new Error(`Presentation skipped requested frame: ${metadata.mediaTime} > ${expectedFrameTime}`));
                if (metadata.mediaTime >= expectedFrameTime - 0.02) resolve(metadata.mediaTime);
                else video.requestVideoFrameCallback(presented);
              };
              video.requestVideoFrameCallback(presented);
            });
            await timeout(new Promise((resolve, reject) => {
              video.onseeked = resolve; video.onerror = reject; video.currentTime = time;
            }));
            let presentedMediaTime;
            try {
              presentedMediaTime = await timeout(presentation);
            } finally { video.pause(); }
            const seekDecodeMs = performance.now() - seekStart;
            const w = video.videoWidth, h = video.videoHeight;
            const canvas = new OffscreenCanvas(w, h);
            const gl = canvas.getContext('webgl2');
            if (!gl) throw new Error('WebGL2 unavailable');
            const texture = gl.createTexture(), fbo = gl.createFramebuffer();
            let premultTexture = null;
            let vf;
            try {
              gl.bindTexture(gl.TEXTURE_2D, texture);
              gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, video);
              gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
              gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
              if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('incomplete framebuffer');
              const rgba = new Uint8Array(w * h * 4);
              gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
              if (gl.getError() !== gl.NO_ERROR) throw new Error('video texture readback failed');
              const two = new OffscreenCanvas(w, h).getContext('2d', { willReadFrequently: true });
              if (!two) throw new Error('Canvas2D unavailable');
              two.drawImage(video, 0, 0);
              premultTexture = gl.createTexture();
              gl.bindTexture(gl.TEXTURE_2D, premultTexture);
              gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
              gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
              try { gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, two.canvas); }
              finally { gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); }
              gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, premultTexture, 0);
              if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('incomplete premultiplied canvas framebuffer');
              const premult = new Uint8Array(w * h * 4);
              gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, premult);
              if (gl.getError() !== gl.NO_ERROR) throw new Error('premultiplied canvas texture readback failed');
              vf = new VideoFrame(video);
              let raw, planes, rawError;
              try { raw = new Uint8Array(vf.allocationSize()); planes = await vf.copyTo(raw); } catch (e) { rawError = String(e); }
              frames.push({ time, presentedMediaTime, seekDecodeMs, w, h, nativeReference: `${row.id}-${frames.length}.premult`,
                rgba: encode(rgba), canvas: encode(two.getImageData(0, 0, w, h).data), premult: encode(premult),
                raw: planes ? encode(raw) : null, planes, rawError, format: vf.format, timestamp: vf.timestamp,
                colorSpace: vf.colorSpace.toJSON() });
            } finally { vf?.close(); gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); gl.deleteTexture(texture); if (premultTexture) gl.deleteTexture(premultTexture); gl.deleteFramebuffer(fbo); gl.getExtension('WEBGL_lose_context')?.loseContext(); }
          }
          return { frames, playback };
        } catch (e) { return { error: String(e) }; }
        finally { video.removeAttribute('src'); video.load(); URL.revokeObjectURL(url); }
      }, { bytes, row, imported });
      if (captured.playback) {
        await writeFile(path.join(destination, captured.playback.file), Buffer.from(captured.playback.bytes, 'base64'));
        captured.playback.sha256 = createHash('sha256').update(Buffer.from(captured.playback.bytes, 'base64')).digest('hex');
        delete captured.playback.bytes;
      }
      for (const [i, frame] of (captured.frames || []).entries()) {
        for (const field of ['rgba', 'canvas', 'premult', 'raw']) {
          if (frame[field]) await writeFile(path.join(destination, `${row.id}-${i}.${field}`), Buffer.from(frame[field], 'base64'));
          delete frame[field];
        }
      }
      report.rows.push({ ...row, ...captured });
      console.log(row.id, captured.error || captured.frames.map(f => `${f.format}@${f.timestamp}`).join(' '));
      await writeFile(path.join(destination, 'chromium.json'), JSON.stringify(report, null, 2));
    }
    const png = await page.evaluate(async () => {
      const canvas = document.createElement('canvas'); canvas.width = 256; canvas.height = 256;
      const context = canvas.getContext('2d'); const data = context.createImageData(256, 256);
      // Every possible premultiplied channel/alpha pair through alphaFrame's
      // truncation and Canvas2D's real putImageData -> PNG encoding path.
      for (let a = 0; a < 256; a++) for (let c = 0; c < 256; c++) {
        const offset = (a * 256 + c) * 4, straight = a ? Math.min(255, Math.floor(c * 255 / a)) : 0;
        data.data.set([straight, straight, straight, a], offset);
      }
      context.putImageData(data, 0, 0);
      const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
      if (!blob) throw new Error('Canvas PNG encode failed');
      return [...new Uint8Array(await blob.arrayBuffer())];
    });
    await writeFile(path.join(destination, 'canvas-alpha-table.png'), new Uint8Array(png));
    await writeFile(path.join(destination, 'versions.json'), JSON.stringify(report.versions, null, 2));
    if (report.rows.some(row => row.error || row.frames?.length !== row.times.length)) process.exitCode = 1;
  } finally {
    try {
      if (app) {
        // An imported project can show an unsaved-changes quit prompt. Do not
        // leave Electron (or the leased lock) waiting behind that dialog.
        const closing = app.close().catch(() => undefined);
        let timer;
        try { await Promise.race([closing, new Promise(resolve => { timer = setTimeout(resolve, 5_000); })]); }
        finally { clearTimeout(timer); }
        if (app.process().exitCode === null) app.process().kill('SIGKILL');
        await closing;
      }
    } finally { await rm(userData, { recursive: true, force: true }); }
  }
}
