import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, launchApp, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';
import { TICKS_PER_SECOND } from '../src/shared/frame-rate';

// Different source and composition rates expose both the old fixed preview
// padding and the old composition-frame export padding. The layer outlasts
// its source, so the final decoded frame must hold on both production routes.
for (const { sourceFps, fps, nested } of [
  { sourceFps: 60, fps: 120, nested: false },
  { sourceFps: 120, fps: 30, nested: true },
]) test(`native viewer and encoded export keep the final ${sourceFps} fps frame at ${fps} fps${nested ? ' through a precomp' : ''}`, async () => {
  test.setTimeout(90_000);
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    const source = path.join(session.userData, 'steps.mp4');
    const output = path.join(session.userData, 'export.mp4');
    const count = Math.ceil(20 / sourceFps * fps) + 3;
    const raw = Buffer.concat(Array.from({ length: 20 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 10)));
    execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', String(sourceFps),
      '-i', 'pipe:0', '-c:v', 'h264_videotoolbox', '-allow_sw', '1', '-b:v', '8M', '-pix_fmt', 'yuv420p',
      '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', source], { input: raw });
    const { page } = session;
    await page.evaluate(({ fps, count }) => {
      const PM = (window as any).PM;
      // Preserve selected presentation surfaces before their pool slots
      // are released; the WebGPU swapchain clears after compositing.
      void PM.Kernel.api('video-endpoint-probe').render.gl.context;
      PM.replaceProject(PM.mkProject({ name: 'Video endpoint', w: 64, h: 64, fps, dur: count / fps }));
      const input = document.createElement('input'); input.type = 'file'; input.id = 'frame-source'; document.body.append(input);
    }, { fps, count });
    await page.locator('#frame-source').setInputFiles(source);
    await page.evaluate(async ({ fps, count, nested }) => {
      const PM = (window as any).PM;
      await PM.importFiles([(document.querySelector('#frame-source') as HTMLInputElement).files![0]]);
      const video = PM.proj.layers.find((layer: any) => layer.type === 'video');
      video.from = 0; video.dur = count / fps; video.d.trim = PM.P(0); video.d.speed = PM.P(1);
      if (nested) {
        const child = PM.mkProject({ name: 'Nested video', w: 64, h: 64, fps: 25, dur: count / fps });
        child.layers = [video]; PM.proj.comps[child.id] = child;
        const precomp = PM.mkLayer('precomp', { from: 0, dur: count / fps, d: { comp: child.id, w: 64, h: 64, trim: 0, speed: 1 } }, PM.proj);
        // Precomp transforms use their top-left origin, unlike video layers.
        precomp.p['position.x'].v = 0; precomp.p['position.y'].v = 0;
        PM.proj.layers = [precomp];
      }
      PM.proj.work = [0, count / fps]; PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
      PM.pause();
    }, { fps, count, nested });

    const samples = new Map<number, number>();
    for (const n of [count - 1, 0, ...Array.from({ length: 5 }, (_, i) => count - 6 + i)]) {
      const time = n / fps;
      await page.evaluate(time => (window as any).PM.setTime(time, { raw: true, force: true }), time);
      await expect.poll(() => page.evaluate(() => {
        const PM = (window as any).PM;
        const view = PM.Kernel.services.get('viewer').enginePresentation;
        const tag = view.latest;
        return [tag?.pts, tag?.viewSeq === Number(view.state.viewSeq),
          !!tag && !!PM.CompatContext.state?.copies.has(`${tag.revision}:${tag.viewSeq}:${tag.seq}`)];
      })).toEqual([Math.round(time * TICKS_PER_SECOND), true, true]);
      const levels = await page.evaluate(async time => {
        const PM = (window as any).PM;
        const presentation = PM.Kernel.services.get('viewer').enginePresentation;
        const state = presentation.stateFor(presentation.latest.viewSeq);
        const tag = presentation.latest;
        const pixels = PM.CompatContext.state?.copies.get(`${tag.revision}:${tag.viewSeq}:${tag.seq}`);
        if (!pixels) throw new Error('The selected viewer surface was not preserved');
        const canvas = document.createElement('canvas'); canvas.width = pixels.width; canvas.height = pixels.height;
        const context = canvas.getContext('2d')!; context.drawImage(pixels, 0, 0);
        const shown = context.getImageData(canvas.width >> 1, canvas.height >> 1, 1, 1).data[0]!;
        const { width, height } = state.canvas;
        const snapshot = await PM.EngineRender.thumbnails.request({
          key: 'video-endpoint', composition: String(PM.proj.compId ?? PM.proj.id), time,
          width, height, pixels: 'opaque', draft3d: true,
          viewer: { view: state.view ?? null, canvas: state.canvas, viewSeq: state.viewSeq },
        });
        return { shown, snapshot: PM.EngineRender.toCanvas(snapshot).getContext('2d').getImageData(width >> 1, height >> 1, 1, 1).data[0] as number };
      }, time);
      const expected = 20 + Math.min(19, Math.floor(time * sourceFps + 1e-6)) * 10;
      for (const [surface, level] of Object.entries(levels)) {
        expect(Math.abs(level - expected), `${surface} frame ${n}: ${level}, expected ${expected}`).toBeLessThan(5);
      }
      samples.set(n, levels.shown);
    }
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, output);
    const result = await page.evaluate(async fps => (window as any).PM.Export.run({
      format: 'mp4', scale: 1, fps, range: 'all', quality: 'high', mblur: false, audio: false,
    }), fps);
    expect(result).toEqual({ cancelled: false });
    const decoded = execFileSync(ffmpegBinary, ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
    expect(decoded.length / 3).toBe(count);
    for (let n = 0; n < count; n++) {
      const expected = 20 + Math.min(19, Math.floor(n / fps * sourceFps + 1e-6)) * 10;
      expect(Math.abs(decoded[n * 3]! - expected), `export frame ${n}: ${decoded[n * 3]}, expected ${expected}`).toBeLessThan(5);
      if (samples.has(n)) expect(Math.abs(decoded[n * 3]! - samples.get(n)!), `preview/export frame ${n}`).toBeLessThan(5);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); }
});
