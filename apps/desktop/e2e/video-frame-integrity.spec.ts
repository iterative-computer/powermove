import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

async function setup(session: any, alpha = false, pattern = false) {
  await session.openEditor();
  const source = path.join(session.userData, 'frame-ramp.webm');
  const raw = Buffer.concat(Array.from({ length: 30 }, (_, i) => {
    const pixels = Buffer.alloc(64 * 64 * (alpha ? 4 : 3), 20 + i * 7);
    if (alpha) for (let offset = 3; offset < pixels.length; offset += 4) pixels[offset] = 128;
    if (pattern) for (let y = 0; y < 64; y++) for (let x = 0; x < 64; x++) {
      const offset = (y * 64 + x) * 4;
      pixels[offset] = (x * 5 + i * 3) % 256; pixels[offset + 1] = (y * 7 + i) % 256;
      pixels[offset + 2] = (x * 3 + y * 2 + 80) % 256;
      pixels[offset + 3] = x < 21 ? 0 : x > 42 ? 255 : 128;
    }
    return pixels;
  }));
  execFileSync(ffmpegBinary, [
    '-v', 'error', '-f', 'rawvideo', '-pix_fmt', alpha ? 'rgba' : 'rgb24', '-s', '64x64', '-r', '10', '-i', 'pipe:0',
    '-c:v', 'libvpx-vp9', '-lossless', '1', '-g', '1', ...(alpha ? ['-pix_fmt', 'yuva420p', '-auto-alt-ref', '0'] : []), source,
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
    layer.from = 0; layer.dur = 3; layer.d.trim = PM.P(0); layer.d.speed = PM.P(1); PM.touch();
    PM.perf.auto = false; PM.quality = 1;
    PM.GL.previewViewport = null; PM.GL.resize(64, 64);
    PM.touch(); PM.bus.emit('layers');
  });
}

test('private video snapshots match direct video uploads for cropped color and alpha without retaining textures', async ({ session }) => {
  await setup(session, true, true);
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0];
    layer.d.w = 144; layer.d.h = 112; layer.d.fit = 'cover';
    layer.p['position.x'].v = 32; layer.p['position.y'].v = 32;
    layer.p['anchor.x'].v = 0; layer.p['anchor.y'].v = 0;
    PM.touch(); PM.bus.emit('layers'); PM.WS?.activate?.('design', true);
  });
  await expect(page.locator('.asset-card.is-comp img.comp-thumb').first()).toBeVisible({ timeout: 15_000 });
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.setTime(.8, { raw: true, force: true });
    await new Promise(resolve => setTimeout(resolve, 250));
    const asset = PM.assets.get(PM.proj.layers[0].d.asset), video = asset.el;
    const state: any = { seeks: 0, video, asset, time: video.currentTime, memory: PM.GL.memoryStats().textures,
      originalRender: PM.GL.renderToPixels, originalMaintain: PM.Memory.maintain, inCapture: false, maintenance: 0, capture: null };
    video.addEventListener('seeking', () => state.seeks++);
    PM.Memory.maintain = (name: string, force: boolean) => {
      if (state.inCapture && name === 'textures') state.maintenance++;
      return state.originalMaintain(name, force);
    };
    PM.GL.renderToPixels = (time: number, width: number, height: number, options: any) => {
      state.inCapture = !!options.isolatedVideoFrames;
      try {
        const pixels = state.originalRender(time, width, height, options);
        if (options.isolatedVideoFrames && options.comp === PM.proj) state.capture = { time, width, height, pixels: Array.from(pixels) };
        return pixels;
      } finally { state.inCapture = false; }
    };
    (window as any).__nativeThumbnailParity = state;
    PM.proj.work = [1, 3]; PM.bus.emit('assets', { runtimeOnly: true });
  });
  await page.waitForFunction(() => (window as any).__nativeThumbnailParity?.capture?.time === 2, null, { timeout: 15_000 });
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM, state = (window as any).__nativeThumbnailParity;
    PM.GL.renderToPixels = state.originalRender; PM.Memory.maintain = state.originalMaintain;
    const ownership = { seeks: state.seeks, before: state.time, after: state.video.currentTime, time: PM.time,
      maintenance: state.maintenance, memoryBefore: state.memory, memoryAfter: PM.GL.memoryStats().textures };
    const previousCapture = PM.agentFrameCapture, previousFrames = PM.preparedVideoFrames;
    PM.preparedVideoFrames = null;
    PM.setTime(2, { raw: true, force: true });
    // Let the ordinary engine move its coalesced seek target as well, so its
    // seek-completion listener cannot pull the reference decoder back to0.8.
    await new Promise(resolve => requestAnimationFrame(resolve));
    PM.agentFrameCapture = true;
    try {
      // Only the reference phase takes ownership of the live decoder, after
      // all viewer-isolation measurements above have already been captured.
      const scale = state.asset.videoTimestampScale;
      const at = Number.isFinite(scale) && scale > 0 && scale <= .001
        ? Math.round(2 / scale + 1e-9) * scale + Math.min(scale / 1000, .000001) : 2;
      await new Promise<void>((resolve, reject) => {
        const cleanup = () => { clearTimeout(timer); state.video.removeEventListener('seeked', ready); };
        const ready = () => {
          if (!state.video.seeking && state.video.readyState >= 2 && Math.abs(state.video.currentTime - at) < .0005) { cleanup(); resolve(); }
        };
        const timer = setTimeout(() => { cleanup(); reject(new Error('Reference frame did not decode')); }, 10_000);
        state.video.addEventListener('seeked', ready); state.video.currentTime = at; ready();
      });
      PM.preparedVideoFrames = null;
      const reference = PM.GL.renderToPixels(2, state.capture.width, state.capture.height, { mblur: false, mbSamples: 1, opaque: true, topDownOpaque: true });
      const captured: number[] = state.capture.pixels;
      let mismatches = 0, maxDelta = 0;
      for (let index = 0; index < captured.length; index++) {
        const delta = Math.abs(captured[index]! - reference[index]); if (delta) mismatches++; maxDelta = Math.max(maxDelta, delta);
      }
      PM.preparedVideoFrames = new Map();
      const beforeMissing = state.video.currentTime;
      let missingFrame = '';
      try { PM.GL.renderToPixels(2.4, state.capture.width, state.capture.height, { mblur: false, opaque: true, isolatedVideoFrames: true }); }
      catch (error) { missingFrame = (error as Error).message; }
      return { ...ownership, mismatches, maxDelta, bytes: captured.length,
        distinctBytes: new Set(captured).size, missingFrame, beforeMissing, afterMissing: state.video.currentTime,
        privateTextures: [...PM.GL.texes.keys()].filter((key: string) => key.startsWith('capture-video:')).length };
    } finally { PM.preparedVideoFrames = previousFrames; PM.agentFrameCapture = previousCapture; PM.setTime(.8, { raw: true, force: true }); }
  });
  expect(result.seeks).toBe(0); expect(result.after).toBe(result.before); expect(result.time).toBe(.8);
  expect(result.maintenance).toBe(0); expect(result.memoryAfter.bytes).toBe(result.memoryBefore.bytes);
  expect(result.memoryAfter.entries).toBe(result.memoryBefore.entries); expect(result.privateTextures).toBe(0);
  expect(result.bytes).toBe(64 * 64 * 4); expect(result.distinctBytes).toBeGreaterThan(32);
  expect(result.mismatches, JSON.stringify(result)).toBe(0); expect(result.maxDelta).toBe(0);
  expect(result.missingFrame).toContain('missing a prepared video frame'); expect(result.afterMissing).toBe(result.beforeMissing);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('motion-blurred video scrubs settle on the requested frame without decoder churn', async ({ session }) => {
  await setup(session);
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.proj.layers[0].mblur = true; PM.touch();
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

for (const variant of [{ name: 'plain', nested: false, alpha: false }, { name: 'nested motion-blurred', nested: true, alpha: false }, { name: 'transparent', nested: false, alpha: true }]) {
test(`${variant.name} composition video thumbnails capture the poster frame without seeking the paused viewer decoder`, async ({ session }) => {
  await setup(session, variant.alpha);
  const { page } = session;
  await page.evaluate(nested => {
    const PM = (window as any).PM;
    if (nested) {
      const video = PM.proj.layers[0]; video.mblur = true;
      const comp = PM.mkProject({ name: 'Nested thumbnail', w: 64, h: 64, fps: 10, dur: 3 }); comp.layers = [video];
      PM.proj.comps = { [comp.id]: comp };
      PM.proj.layers = [PM.mkLayer('precomp', { d: { comp: comp.id, w: 64, h: 64, trim: 0, speed: 1 },
        p: { 'position.x': 32, 'position.y': 32, 'anchor.x': 32, 'anchor.y': 32 } })];
      // The factory resets precomp positions; apply the authored placement afterwards.
      PM.proj.layers[0].p['position.x'].v = 32; PM.proj.layers[0].p['position.y'].v = 32;
      PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    }
    PM.WS?.activate?.('design', true);
  }, variant.nested);
  const thumbnail = page.locator('.asset-card.is-comp img.comp-thumb').first();
  await expect(thumbnail).toBeVisible({ timeout: 15_000 });
  const previous = await thumbnail.getAttribute('src');
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.setTime(.8, { raw: true, force: true });
    await new Promise(resolve => setTimeout(resolve, 250));
    const layer = PM.proj.layers.find((item: any) => item.type === 'video') || Object.values(PM.proj.comps).flatMap((comp: any) => comp.layers).find((item: any) => item.type === 'video');
    const video = PM.assets.get(layer.d.asset).el;
    const pixel = new Uint8Array(4), gl = PM.GL.gl;
    gl.readPixels(32, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    const state = { seeks: 0, start: video.currentTime, video, viewerLevel: pixel[0], memory: PM.GL.memoryStats().textures };
    video.addEventListener('seeking', () => state.seeks++);
    (window as any).__thumbnailVideoGuard = state;
    PM.proj.work = [1, 3]; // Poster time is2s, distinct from the visible0.8s.
    PM.bus.emit('assets', { runtimeOnly: true });
  });
  await expect.poll(() => thumbnail.getAttribute('src'), { timeout: 15_000 }).not.toBe(previous);
  const result = await thumbnail.evaluate(async (img: HTMLImageElement) => {
    await img.decode();
    const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
    const context = canvas.getContext('2d')!; context.drawImage(img, 0, 0);
    const level = context.getImageData(canvas.width >> 1, canvas.height >> 1, 1, 1).data[0];
    const PM = (window as any).PM, guard = (window as any).__thumbnailVideoGuard;
    return { level, viewerLevel: guard.viewerLevel, seeks: guard.seeks, before: guard.start, after: guard.video.currentTime, time: PM.time,
      memoryBefore: guard.memory, memoryAfter: PM.GL.memoryStats().textures,
      privateTextures: [...PM.GL.texes.keys()].filter((key: string) => key.startsWith('capture-video:')).length };
  });
  expect(Math.abs(result.level! - (variant.alpha ? 80 : 160)), JSON.stringify(result)).toBeLessThan(4);
  expect(Math.abs(result.viewerLevel - (variant.alpha ? 38 : 76)), JSON.stringify(result)).toBeLessThan(4);
  expect(result.seeks).toBe(0); expect(result.after).toBe(result.before); expect(result.time).toBe(.8);
  expect(result.privateTextures).toBe(0); expect(result.memoryAfter.bytes).toBe(result.memoryBefore.bytes);
  expect(result.memoryAfter.entries).toBe(result.memoryBefore.entries);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
}

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
    PM.proj.w = 128; PM.touch();
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
