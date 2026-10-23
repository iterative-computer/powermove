/** Capture the shipped Chromium decoder and texImage2D paths in a hidden app.
 * Build the app first. Generate outside the Electron lease, then capture under
 * pm-electron-lock. All media and evidence must live outside the repository.
 *   node scripts/video-parity.mjs generate /absolute/evidence
 *   pm-electron-lock node scripts/video-parity.mjs capture /absolute/evidence
 */
import { mkdir, readFile, writeFile, mkdtemp, rm, access } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(desktop, '../..');
const [mode, destination] = process.argv.slice(2);
if (!['generate', 'capture', 'png'].includes(mode) || !destination || !path.isAbsolute(destination)
    || path.resolve(destination).startsWith(root + path.sep)) {
  throw new Error('usage: video-parity.mjs generate|capture|png ABSOLUTE_DIRECTORY_OUTSIDE_REPO');
}
await mkdir(destination, { recursive: true });
const recipes = [
  ['h264-601-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-709-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-709-matrix-only', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_range', 'tv']],
  ['h264-hd-709-matrix-only', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_range', 'tv']],
  ['h264-709-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['h264-2020-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-709-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['vp9-709-limited', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'realtime', '-cpu-used', '8'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['vp9-untagged', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'realtime', '-cpu-used', '8'], []],
  ['av1-709-limited', 'mp4', ['libsvtav1', '-pix_fmt', 'yuv420p', '-preset', '12', '-svtav1-params', 'lp=1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['prores422-709', 'mov', ['prores_ks', '-profile:v', '2', '-pix_fmt', 'yuv422p10le'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['prores4444-709', 'mov', ['prores_ks', '-profile:v', '4', '-pix_fmt', 'yuva444p10le'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
];
if (mode === 'generate') {
  const rows = [];
  for (const [id, ext, codec, tags] of recipes) {
    const file = `${id}.${ext}`;
    const bitstream = id.startsWith('h264-') && !id.endsWith('matrix-only')
      ? ['-bsf:v', `h264_metadata=colour_primaries=${id.includes('601') ? 6 : id.includes('2020') ? 9 : 1}:transfer_characteristics=1:matrix_coefficients=${id.includes('601') ? 6 : id.includes('2020') ? 9 : 1}:video_full_range_flag=${id.endsWith('full') ? 1 : 0}`]
      : [];
    // Detail, gradients, motion and midtones in every codec; deliberately not
    // only saturated patches, which hide reconstruction and rounding errors.
    execFileSync(process.env.PM_PARITY_FFMPEG || 'ffmpeg', ['-v', 'error', '-y', '-f', 'lavfi', '-i',
      `testsrc2=size=${id.includes('-hd-') ? '1280x720' : '256x144'}:rate=12:duration=1.5`, '-vf',
      "geq=lum='32+0.65*lum(X,Y)+20*X/W':cb='128+0.65*(cb(X,Y)-128)':cr='128+0.65*(cr(X,Y)-128)'",
      '-filter_threads', '1', '-threads', '1', '-c:v', ...codec, ...tags, ...bitstream, '-an', path.join(destination, file)],
    { stdio: 'inherit' });
    const probe = JSON.parse(execFileSync(process.env.PM_PARITY_FFPROBE || 'ffprobe', ['-v', 'error', '-show_streams', '-of', 'json', path.join(destination, file)], { encoding: 'utf8' }));
    rows.push({ id, file, times: [0.125, 0.625, 1.125], probe: probe.streams[0] });
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
    const rows = mode === 'png' ? [] : JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
    const report = { versions: await app.evaluate(() => process.versions), rows: [] };
    for (const row of rows) {
      const bytes = [...await readFile(path.join(destination, row.file))];
      const captured = await page.evaluate(async ({ bytes, row }) => {
        const encode = bytes => {
          let binary = '';
          for (let offset = 0; offset < bytes.length; offset += 8192) binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
          return btoa(binary);
        };
        const video = document.createElement('video');
        video.muted = true; video.preload = 'auto';
        const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)]));
        const timeout = (promise) => {
          let timer;
          return Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('video timeout')), 15_000); })])
            .finally(() => clearTimeout(timer));
        };
        try {
          await timeout(new Promise((resolve, reject) => {
            video.onloadeddata = resolve; video.onerror = () => reject(new Error(`HTMLVideoElement error ${video.error?.code}: ${video.error?.message}`));
            video.src = url;
          }));
          const frames = [];
          for (const time of row.times) {
            await timeout(new Promise((resolve, reject) => {
              video.onseeked = resolve; video.onerror = reject; video.currentTime = time;
            }));
            const w = video.videoWidth, h = video.videoHeight;
            const canvas = new OffscreenCanvas(w, h);
            const gl = canvas.getContext('webgl2');
            if (!gl) throw new Error('WebGL2 unavailable');
            const texture = gl.createTexture(), fbo = gl.createFramebuffer();
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
              two.drawImage(video, 0, 0);
              vf = new VideoFrame(video);
              let raw, planes, rawError;
              try { raw = new Uint8Array(vf.allocationSize()); planes = await vf.copyTo(raw); } catch (e) { rawError = String(e); }
              frames.push({ time, w, h, rgba: encode(rgba), canvas: encode(two.getImageData(0, 0, w, h).data),
                raw: planes ? encode(raw) : null, planes, rawError, format: vf.format, timestamp: vf.timestamp,
                colorSpace: vf.colorSpace.toJSON() });
            } finally { vf?.close(); gl.deleteTexture(texture); gl.deleteFramebuffer(fbo); gl.getExtension('WEBGL_lose_context')?.loseContext(); }
          }
          return { frames };
        } catch (e) { return { error: String(e) }; }
        finally { video.removeAttribute('src'); video.load(); URL.revokeObjectURL(url); }
      }, { bytes, row });
      for (const [i, frame] of (captured.frames || []).entries()) {
        for (const field of ['rgba', 'canvas', 'raw']) {
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
  } finally {
    try { await app?.close(); } finally { await rm(userData, { recursive: true, force: true }); }
  }
}
