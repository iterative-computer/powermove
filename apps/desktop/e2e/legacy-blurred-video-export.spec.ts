import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/*
 * Which video frame today's engine exports for a motion-blurred video layer
 * (docs/engine/motion.md, "Video in blurred samples"). The corpus capture
 * draws a stale, unseeked frame; this records what an MP4 export of the
 * legacy engine contains, frame by frame, for a static blurred clip whose
 * frames are flat levels (20 + 10 i).
 */
for (const mblur of [false, true]) test(`legacy export of a ${mblur ? 'motion-blurred' : 'sharp'} video layer`, async ({ session }, info) => {
  await session.openEditor();
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-blurred-video-'));
  try {
    const fps = 30;
    const source = path.join(root, 'steps.webm');
    const raw = Buffer.concat(Array.from({ length: 20 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 10)));
    execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', String(fps), '-i', 'pipe:0',
      '-c:v', 'libvpx-vp9', '-lossless', '1', source], { input: raw });
    await session.page.evaluate(fps => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Blurred video', w: 64, h: 64, fps, dur: 20 / fps });
      PM.proj.work = [0, 20 / fps]; PM.bus.emit('project');
      const input = document.createElement('input'); input.type = 'file'; input.id = 'frame-source'; document.body.appendChild(input);
    }, fps);
    await session.page.locator('#frame-source').setInputFiles(source);
    const output = path.join(root, 'export.mp4');
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, output);
    const result = await session.page.evaluate(async ({ fps, mblur }) => {
      const PM = (window as any).PM;
      await PM.importFiles([(document.querySelector('#frame-source') as HTMLInputElement).files![0]]);
      const layer = PM.proj.layers.find((l: any) => l.type === 'video'); layer.from = 0; layer.dur = 20 / fps; layer.mblur = mblur; PM.touch();
      PM.setTime(15 / fps, { force: true });
      await new Promise(resolve => setTimeout(resolve, 300));
      const result = await PM.Export.run({ format: 'mp4', scale: 1, fps, range: 'all', quality: 'high', mblur, audio: false });
      if (result.error) throw new Error(result.error);
      return { result, engine: PM.engineMode ?? null };
    }, { fps, mblur });
    expect(result.result).toEqual({ cancelled: false });
    const decoded = execFileSync(ffmpegBinary, ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
    const levels = Array.from({ length: decoded.length / 3 }, (_, i) => decoded[i * 3]);
    const expected = levels.map((_, i) => 20 + i * 10);
    const report = JSON.stringify({ mblur, levels, expected });
    console.log(`blurred-video-export ${report}`);
    await info.attach('levels', { body: report, contentType: 'application/json' });
    expect(levels.length).toBe(20);
    if (!mblur) for (let i = 0; i < levels.length; i++) expect(Math.abs(levels[i]! - expected[i]!), report).toBeLessThan(5);
  } finally { await rm(root, { recursive: true, force: true }); }
});

/*
 * The same clip in the paused viewer (today's preview path,
 * `useVideoPreviews`): for each frame, the playhead is set, the viewer's
 * video previews are given time to seek, and the viewer's canvas is read
 * right after a render (the drawing buffer is valid within the task).
 */
test('legacy paused preview of a motion-blurred video layer', async ({ session }, info) => {
  await session.openEditor();
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-blurred-preview-'));
  try {
    const fps = 30;
    const source = path.join(root, 'steps.webm');
    const raw = Buffer.concat(Array.from({ length: 20 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 10)));
    execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', String(fps), '-i', 'pipe:0',
      '-c:v', 'libvpx-vp9', '-lossless', '1', source], { input: raw });
    await session.page.evaluate(fps => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Blurred preview', w: 64, h: 64, fps, dur: 20 / fps });
      PM.proj.work = [0, 20 / fps]; PM.bus.emit('project');
      const input = document.createElement('input'); input.type = 'file'; input.id = 'frame-source'; document.body.appendChild(input);
    }, fps);
    await session.page.locator('#frame-source').setInputFiles(source);
    const levels = await session.page.evaluate(async ({ fps }) => {
      const PM = (window as any).PM;
      await PM.importFiles([(document.querySelector('#frame-source') as HTMLInputElement).files![0]]);
      const layer = PM.proj.layers.find((l: any) => l.type === 'video'); layer.from = 0; layer.dur = 20 / fps; layer.mblur = true; PM.touch();
      const out: Array<{ sharp: number; blurred: number }> = [];
      const read = (t: number) => {
        PM.GL.render(t);
        const gl = PM.GL.gl as WebGL2RenderingContext;
        const px = new Uint8Array(4);
        gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        return px[0]!;
      };
      for (const frame of [3, 8, 15]) {
        const t = frame / fps;
        layer.mblur = false; PM.touch();
        PM.setTime(t, { force: true });
        await new Promise(resolve => setTimeout(resolve, 600));
        const sharp = read(t);
        layer.mblur = true; PM.touch();
        PM.setTime(t, { force: true });
        await new Promise(resolve => setTimeout(resolve, 600));
        out.push({ sharp, blurred: read(t) });
      }
      return out;
    }, { fps });
    const report = JSON.stringify({ frames: [3, 8, 15], expected: [50, 100, 170], levels });
    console.log(`blurred-video-preview ${report}`);
    await info.attach('levels', { body: report, contentType: 'application/json' });
  } finally { await rm(root, { recursive: true, force: true }); }
});
