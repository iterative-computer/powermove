import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

async function setup(session: any) {
  await session.openEditor();
  const source = path.join(session.userData, 'frame-ramp.webm');
  const raw = Buffer.concat(Array.from({ length: 30 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 7)));
  execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), [
    '-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', '10', '-i', 'pipe:0',
    '-c:v', 'libvpx-vp9', '-lossless', '1', '-g', '1', source,
  ], { input: raw });
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.proj = PM.mkProject({ name: 'Frame integrity', w: 64, h: 64, fps: 10, dur: 3 });
    PM.proj.work = [0, 3]; PM.bus.emit('project');
    const input = document.createElement('input'); input.type = 'file'; input.id = 'frame-source'; document.body.appendChild(input);
  });
  await page.locator('#frame-source').setInputFiles(source);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.importFiles([(document.querySelector('#frame-source') as HTMLInputElement).files![0]]);
    const layer = PM.proj.layers.find((l: any) => l.type === 'video');
    layer.from = 0; layer.dur = 3; layer.d.trim = PM.P(0); layer.d.speed = PM.P(1);
    PM.perf.auto = false; PM.quality = 1;
    PM.GL.previewViewport = null; PM.GL.resize(64, 64);
    PM.touch(); PM.bus.emit('layers');
  });
}

test('motion-blurred video scrubs settle on the requested frame without decoder churn', async ({ session }) => {
  await setup(session);
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.proj.layers[0].mblur = true;
    const video = PM.assets.get(PM.proj.layers[0].d.asset).el;
    let seeks = 0;
    video.addEventListener('seeking', () => seeks++);
    const samples: { time: number; level: number; seeks: number }[] = [];
    for (const time of [1.4, .3, 2.1, .8]) {
      PM.setTime(time, { raw: true, force: true });
      await new Promise(r => setTimeout(r, 200));
      const before = seeks;
      for (let i = 0; i < 6; i++) {
        await new Promise(r => setTimeout(r, 35));
        PM.GL.render(time, { mblur: true, mbSamples: 6, shutter: .5 });
        const gl = PM.GL.gl, pixel = new Uint8Array(4);
        gl.readPixels(32, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
        samples.push({ time, level: pixel[0]!, seeks: seeks - before });
      }
    }
    return samples;
  });
  for (const sample of result) {
    expect(Math.abs(sample.level - (20 + Math.round(sample.time * 10) * 7)), JSON.stringify(sample)).toBeLessThan(4);
    expect(sample.seeks, JSON.stringify(sample)).toBeLessThanOrEqual(1);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('repeated trimmed cuts and loops never present a frame from the previous clip', async ({ session }) => {
  await setup(session);
  const samples = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    const base = PM.proj.layers[0];
    PM.proj.layers = [1.5, 0, 2].map((trim, i) => {
      const layer = structuredClone(base); layer.id = PM.uid('l');
      layer.from = i * .5; layer.dur = .5; layer.d.trim = PM.P(trim); layer.mblur = true;
      return layer;
    });
    PM.proj.work = [0, 1.5]; PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.setTime(0, { force: true }); await new Promise(r => setTimeout(r, 200));
    const original = PM.GL.render, samples: { time: number; level: number }[] = [];
    PM.GL.render = (time: number, options: any) => {
      const result = original(time, options);
      if (result !== false && PM.playing) {
        const gl = PM.GL.gl, pixel = new Uint8Array(4);
        gl.readPixels(32, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
        samples.push({ time, level: pixel[0]! });
      }
      return result;
    };
    try { PM.play(); await new Promise(r => setTimeout(r, 3300)); }
    finally { PM.pause(); PM.GL.render = original; }
    return samples;
  });
  expect(samples.length).toBeGreaterThan(20);
  for (const { time, level } of samples) {
    const index = Math.min(2, Math.floor(time / .5));
    const sourceTime = [1.5, 0, 2][index]! + time - index * .5;
    expect(Math.abs(level - (20 + Math.floor(sourceTime * 10 + 1e-6) * 7)), JSON.stringify({ time, level, sourceTime })).toBeLessThan(18);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('two instances of a nested video composition keep separate playback times', async ({ session }) => {
  await setup(session);
  const samples = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    const video = PM.proj.layers[0];
    const sub = PM.mkProject({ name: 'Nested', w: 64, h: 64, fps: 10, dur: 3 });
    sub.layers = [video]; PM.proj.comps = { [sub.id]: sub };
    PM.proj.w = 128;
    PM.proj.layers = [0, 1].map(i => {
      const layer = PM.mkLayer('precomp', { d: { comp: sub.id, w: 64, h: 64, trim: i, speed: 1 }, p: { 'position.x': i * 64, 'position.y': 0 } });
      layer.p['position.x'].v = i * 64;
      layer.from = 0; layer.dur = 1; return layer;
    });
    PM.proj.work = [0, 1]; PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.GL.resize(128, 64); PM.setTime(.2, { force: true });
    await new Promise(r => setTimeout(r, 250));
    const original = PM.GL.render, samples: { time: number; levels: number[] }[] = [];
    PM.GL.render = (time: number, options: any) => {
      const result = original(time, options);
      if (result !== false && PM.playing) {
        const levels = [32, 96].map(x => {
          const gl = PM.GL.gl, pixel = new Uint8Array(4); gl.readPixels(x, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel); return pixel[0]!;
        });
        samples.push({ time, levels });
      }
      return result;
    };
    try { PM.play(); await new Promise(r => setTimeout(r, 650)); }
    finally { PM.pause(); PM.GL.render = original; }
    return samples;
  });
  expect(samples.length).toBeGreaterThan(3);
  for (const sample of samples) {
    expect(Math.abs(sample.levels[1]! - sample.levels[0]! - 70), JSON.stringify(sample)).toBeLessThan(10);
  }
  expect(samples.at(-1)!.levels[0]!).toBeGreaterThan(samples[0]!.levels[0]! + 14);
});
