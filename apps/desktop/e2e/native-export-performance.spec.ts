import { writeFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot, type LaunchedApp } from './helpers/app';

for (const alpha of [false, true]) {
  test(`native export preserves every input pixel (${alpha ? 'alpha' : 'opaque'})`, async ({ session }) => {
    await session.openEditor();
    await session.app.evaluate(({ ipcMain }) => {
      const state = { chunks: [] as number[][], finished: false };
      (globalThis as any).__exportPixels = state;
      for (const name of ['start', 'write', 'finish', 'cancel']) ipcMain.removeHandler(`render:${name}`);
      ipcMain.handle('render:start', () => 'pixel-test');
      ipcMain.handle('render:write', (_e, request) => { state.chunks.push(Array.from(request.data)); });
      ipcMain.handle('render:finish', () => { state.finished = true; return { path: '/test/pixels' }; });
      ipcMain.handle('render:cancel', () => {});
    });
    const expected = await session.page.evaluate(async alpha => {
      const PM = (window as any).PM;
      PM.pause(); PM.agentFrameCapture = true;
      const project = PM.mkProject({ name: 'Native pixel parity', w: 96, h: 64, dur: 1, fps: 6 });
      project.backgroundFill = { type: 'none', stops: [], angle: 0 };
      const shape = PM.mkLayer('shape', { d: { w: 69, h: 43, radius: 11, color: '#a34bf7' }, p: { 'position.x': 38, 'position.y': 25, opacity: 37, rotation: 17 } }, project);
      const bottom = PM.mkLayer('shape', { d: { shape: 'ellipse', w: 41, h: 39, color: '#2bd773' }, p: { 'position.x': 70, 'position.y': 45, opacity: 81 } }, project);
      project.layers = [shape, bottom]; PM.proj = project; PM.touch();
      PM.animate(shape, 'position.x', [{ t: 0, v: 18 }, { t: 1, v: 77 }]);
      const expected: number[] = [];
      // Independent reference matching the established canvas delivery route.
      for (let frame = 0; frame < 6; frame++) {
        const t = frame / 6;
        const pixels = PM.GL.renderToPixels(t, 96, 64, { opaque: !alpha, transparent: alpha, mblur: false, mbSamples: alpha ? 20 : 1, shutter: .5 });
        const canvas = document.createElement('canvas'); canvas.width = 96; canvas.height = 64;
        const ctx = canvas.getContext('2d')!, image = ctx.createImageData(96, 64);
        for (let y = 0; y < 64; y++) for (let x = 0; x < 96; x++) {
          const src = ((63 - y) * 96 + x) * 4, dst = (y * 96 + x) * 4, a = pixels[src + 3];
          for (let c = 0; c < 3; c++) image.data[dst+c] = !alpha || a === 255 ? pixels[src+c]
            : a === 0 ? 0 : Math.min(255, (pixels[src+c] * 255 / a) | 0);
          image.data[dst+3] = alpha ? a : 255;
        }
        ctx.putImageData(image, 0, 0);
        expected.push(...ctx.getImageData(0, 0, 96, 64).data);
      }
      const serialized = JSON.stringify(project), time = PM.time, quality = PM.quality;
      const original = PM.GL.createOpaqueReadback, targets = new Set<Uint8Array>();
      const readback = { sessions: 0, begins: 0, reads: 0, disposed: 0, targetBytes: 0 };
      PM.GL.createOpaqueReadback = (...args: any[]) => {
        const session = original(...args);
        if (!session) return session;
        readback.sessions++;
        return {
          begin(...args: any[]) {
            const ticket = session.begin(...args); readback.begins++;
            return {
              readInto(target: Uint8Array, cancelled: () => boolean) {
                targets.add(target); readback.reads++; readback.targetBytes = target.byteLength;
                return ticket.readInto(target, cancelled);
              },
              dispose() { ticket.dispose(); },
            };
          },
          dispose() { readback.disposed++; session.dispose(); },
        };
      };
      try {
        const result = await PM.Export.run({ format: alpha ? 'prores' : 'mp4', alpha, scale: 1, fps: 6, range: 'all', quality: 'draft', audio: false, mblur: false });
        return { expected, result, readback: { ...readback, targets: targets.size },
          preserved: serialized === JSON.stringify(project) && time === PM.time && quality === PM.quality };
      } finally { PM.GL.createOpaqueReadback = original; }
    }, alpha);
    const actual = await session.app.evaluate(() => (globalThis as any).__exportPixels);
    expect(expected.result).toEqual({ cancelled: false });
    expect(expected.preserved).toBe(true);
    expect(expected.readback).toEqual(alpha
      ? { sessions: 0, begins: 0, reads: 0, disposed: 0, targetBytes: 0, targets: 0 }
      : { sessions: 1, begins: 6, reads: 6, disposed: 1, targetBytes: 96 * 64 * 4, targets: 1 });
    expect(actual.finished).toBe(true);
    expect(actual.chunks.flat()).toEqual(expected.expected);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

test('native opaque export rejects context loss during asynchronous readback and retries with exact pixels', async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ ipcMain, dialog }, output) => {
    const state = { writes: 0, finishes: 0, cancels: 0, chunks: [] as number[][] };
    (globalThis as any).__asyncCaptureRecovery = state;
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
    for (const name of ['write', 'finish', 'cancel']) {
      const channel = `render:${name}`, original = (ipcMain as any)._invokeHandlers.get(channel);
      ipcMain.removeHandler(channel);
      ipcMain.handle(channel, async (...args: any[]) => {
        if (name === 'write') { state.writes++; state.chunks.push(Array.from(args[1].data)); }
        if (name === 'finish') state.finishes++;
        if (name === 'cancel') state.cancels++;
        return await original(...args);
      });
    }
  }, path.join(session.userData, 'gpu-readback-recovery.mp4'));
  const reference = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Asynchronous GPU recovery', w: 64, h: 64, fps: 4, dur: 1, bg: '#21344a' });
    const shape = PM.mkLayer('shape', { d: { w: 31, h: 43, radius: 9, color: '#dd7355' },
      p: { 'position.x': 20, 'position.y': 30, opacity: 63 } }, project);
    project.layers = [shape]; PM.proj = project; PM.touch();
    PM.animate(shape, 'position.x', [{ t: 0, v: 12 }, { t: 1, v: 48 }]);
    (window as any).__asyncNativeGpuLoss = PM.GL.gl.getExtension('WEBGL_lose_context');
    const pixels: number[] = [];
    for (let i = 0; i < 4; i++) pixels.push(...PM.GL.renderToPixels(i / 4, 64, 64,
      { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 }));
    return { supported: !!(window as any).__asyncNativeGpuLoss, pixels };
  });
  test.skip(!reference.supported, 'GPU reset extension unavailable');
  const failed = await session.page.evaluate(async () => {
    const PM = (window as any).PM, original = PM.GL.createOpaqueReadback;
    let reads = 0;
    PM.GL.createOpaqueReadback = (...args: any[]) => {
      const session = original(...args);
      if (!session) return session;
      return {
        begin(...args: any[]) {
          const ticket = session.begin(...args);
          return {
            readInto(target: Uint8Array, cancelled: () => boolean) {
              const reading = ticket.readInto(target, cancelled);
              reads++;
              (window as any).__asyncNativeGpuLoss.loseContext();
              return reading;
            },
            dispose() { ticket.dispose(); },
          };
        },
        dispose() { session.dispose(); },
      };
    };
    try {
      const result = await PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4, range: 'all', quality: 'draft', audio: false, mblur: false });
      return { result, reads, busy: PM.Export.busy };
    } finally { PM.GL.createOpaqueReadback = original; }
  });
  expect(failed).toEqual({ result: { error: 'The GPU could not capture the export frame' }, reads: 1, busy: false });
  expect(await session.app.evaluate(() => (globalThis as any).__asyncCaptureRecovery)).toEqual({ writes: 0, finishes: 0, cancels: 1, chunks: [] });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.GL.contextLost)).toBe(true);
  await session.page.evaluate(() => (window as any).__asyncNativeGpuLoss.restoreContext());
  await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.GL.gl && !(window as any).PM.GL.contextLost)).toBe(true);
  expect(await session.page.evaluate(async () => (window as any).PM.Export.run({
    format: 'mp4', alpha: false, scale: 1, fps: 4, range: 'all', quality: 'draft', audio: false, mblur: false,
  }))).toEqual({ cancelled: false });
  const recovered = await session.app.evaluate(() => (globalThis as any).__asyncCaptureRecovery);
  expect(recovered).toMatchObject({ writes: 4, finishes: 1, cancels: 1 });
  expect(recovered.chunks.flat()).toEqual(reference.pixels);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('profiles full native MP4 delivery at HD and 4K', async ({ session }, info) => {
  test.setTimeout(120_000);
  await session.openEditor();
  await session.app.evaluate(({ dialog }, root) => {
    let sequence = 0;
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: `${root}/profile-${sequence++}.mp4` });
  }, session.userData);
  const results = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const results: any[] = [];
    for (const [width, height, frames] of [[1920, 1080, 24], [3840, 2160, 12]]) {
      const project = PM.mkProject({ name: 'Native profile', w: width, h: height, dur: 1, fps: frames, bg: '#21344a' });
      const shape = PM.mkLayer('shape', { d: { w: width / 3, h: height / 2, radius: 27, color: '#dd7355' }, p: { 'position.x': width / 2, 'position.y': height / 2, opacity: 63 } }, project);
      project.layers = [shape]; PM.proj = project; PM.touch();
      PM.animate(shape, 'rotation', [{ t: 0, v: 0 }, { t: 1, v: 50 }]);
      for (let warm = 0; warm < 3; warm++) PM.renderFrameTo(warm / frames, width, height, { mblur: false });
      for (let run = 0; run < 3; run++) {
        const originalRead = CanvasRenderingContext2D.prototype.getImageData;
        const originalPut = CanvasRenderingContext2D.prototype.putImageData;
        const originalPixels = PM.GL.renderToPixels;
        let reads = 0, readMs = 0, putMs = 0, gpuMs = 0;
        CanvasRenderingContext2D.prototype.getImageData = function (...args: any[]) {
          const start = performance.now(); const result = (originalRead as any).apply(this, args);
          if (args[2] === width && args[3] === height) { reads++; readMs += performance.now() - start; }
          return result;
        };
        CanvasRenderingContext2D.prototype.putImageData = function (...args: any[]) {
          const start = performance.now(); const result = (originalPut as any).apply(this, args);
          if (args[0].width === width && args[0].height === height) putMs += performance.now() - start;
          return result;
        };
        PM.GL.renderToPixels = (...args: any[]) => { const start = performance.now(); const result = originalPixels(...args); gpuMs += performance.now() - start; return result; };
        try {
          const start = performance.now();
          const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: frames, range: 'all', quality: 'draft', alpha: false, audio: false, mblur: false });
          results.push({ width, height, frames, run, ms: performance.now() - start, reads, readMs, putMs, gpuMs, result });
        } finally {
          CanvasRenderingContext2D.prototype.getImageData = originalRead;
          CanvasRenderingContext2D.prototype.putImageData = originalPut;
          PM.GL.renderToPixels = originalPixels;
        }
      }
    }
    return results;
  });
  console.log('NATIVE_EXPORT', JSON.stringify(results));
  writeFileSync(info.outputPath('native-export-metrics.json'), JSON.stringify(results, null, 2));
  if (process.env.PM_NATIVE_EXPORT_METRICS) writeFileSync(path.resolve(process.env.PM_NATIVE_EXPORT_METRICS), JSON.stringify(results, null, 2));
  for (const result of results) {
    expect(result.result).toEqual({ cancelled: false });
    if (process.env.PM_NATIVE_EXPORT_BASELINE !== '1') expect(result.reads).toBe(0);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const alpha of [false, true]) test(`native ${alpha ? 'alpha' : 'opaque'} export rejects a lost GPU frame and succeeds after recovery`, async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ ipcMain }) => {
    const state = { writes: 0, finishes: 0, cancels: 0 };
    (globalThis as any).__captureRecovery = state;
    for (const name of ['start', 'write', 'finish', 'cancel']) ipcMain.removeHandler(`render:${name}`);
    ipcMain.handle('render:start', () => 'gpu-recovery');
    ipcMain.handle('render:write', () => { state.writes++; });
    ipcMain.handle('render:finish', () => { state.finishes++; return { path: '/test/recovered' }; });
    ipcMain.handle('render:cancel', () => { state.cancels++; });
  });
  const supported = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    PM.proj = PM.mkProject({ name: 'Native GPU recovery', w: 64, h: 64, fps: 2, dur: 1, bg: '#9b54c3' }); PM.touch();
    (window as any).__nativeGpuLoss = PM.GL.gl.getExtension('WEBGL_lose_context');
    return !!(window as any).__nativeGpuLoss;
  });
  test.skip(!supported, 'GPU reset extension unavailable');
  const failed = await session.page.evaluate(async alpha => {
    const PM = (window as any).PM, original = PM.GL.renderToPixels;
    PM.GL.renderToPixels = (...args: any[]) => {
      const pixels = original(...args);
      (window as any).__nativeGpuLoss.loseContext();
      return pixels;
    };
    try { return await PM.Export.run({ format: alpha ? 'prores' : 'mp4', alpha, scale: 1, fps: 2, range: 'all', quality: 'draft', audio: false, mblur: false }); }
    finally { PM.GL.renderToPixels = original; }
  }, alpha);
  expect(failed).toEqual({ error: 'The GPU could not capture the export frame' });
  expect(await session.app.evaluate(() => (globalThis as any).__captureRecovery)).toEqual({ writes: 0, finishes: 0, cancels: 1 });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.GL.contextLost)).toBe(true);
  await session.page.evaluate(() => (window as any).__nativeGpuLoss.restoreContext());
  await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.GL.gl && !(window as any).PM.GL.contextLost)).toBe(true);
  const retry = await session.page.evaluate(async alpha => {
    const PM = (window as any).PM;
    return await PM.Export.run({ format: alpha ? 'prores' : 'mp4', alpha, scale: 1, fps: 2, range: 'all', quality: 'draft', audio: false, mblur: false });
  }, alpha);
  expect(retry).toEqual({ cancelled: false });
  expect(await session.app.evaluate(() => (globalThis as any).__captureRecovery)).toEqual({ writes: 2, finishes: 1, cancels: 1 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

async function readyNativeMedia(session: LaunchedApp, kind: 'image' | 'video') {
  let encoded: string | undefined;
  if (kind === 'video') {
    const source = path.join(session.userData, 'native-ready-source.mp4');
    const raw = Buffer.concat(Array.from({ length: 8 }, (_, frame) => {
      const pixels = Buffer.alloc(96 * 64 * 3);
      for (let y = 0; y < 64; y++) for (let x = 0; x < 96; x++) {
        const at = (y * 96 + x) * 3;
        pixels[at] = (x * 3 + frame * 23) % 256;
        pixels[at + 1] = (y * 4 + frame * 17) % 256;
        pixels[at + 2] = (x + y + frame * 31) % 256;
      }
      return pixels;
    }));
    execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), ['-v', 'error', '-f', 'rawvideo',
      '-pix_fmt', 'rgb24', '-s', '96x64', '-r', '4', '-i', 'pipe:0', '-c:v', 'libx264', '-crf', '0',
      '-g', '1', '-pix_fmt', 'yuv420p', source], { input: raw });
    encoded = (await readFile(source)).toString('base64');
  }
  return session.page.evaluate(async ({ kind, encoded }) => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.AutomaticPreview?.cancel();
    let blob: Blob;
    if (kind === 'image') {
      const canvas = document.createElement('canvas'); canvas.width = 96; canvas.height = 64;
      const ctx = canvas.getContext('2d')!, gradient = ctx.createLinearGradient(0, 0, 96, 64);
      gradient.addColorStop(0, '#f6427a'); gradient.addColorStop(1, '#2ed9b5');
      ctx.fillStyle = gradient; ctx.fillRect(0, 0, 96, 64);
      ctx.fillStyle = '#2b47d3'; ctx.fillRect(18, 13, 47, 31);
      blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!)));
    } else blob = new Blob([Uint8Array.from(atob(encoded!), char => char.charCodeAt(0))], { type: 'video/mp4' });
    const url = URL.createObjectURL(blob);
    let el: ImageBitmap | HTMLVideoElement;
    if (kind === 'image') el = await createImageBitmap(blob);
    else {
      const video = document.createElement('video'); video.muted = true; video.preload = 'auto';
      await new Promise<void>((resolve, reject) => {
        video.addEventListener('loadeddata', () => resolve(), { once: true });
        video.addEventListener('error', () => reject(new Error('Native video fixture did not load')), { once: true });
        video.src = url;
      });
      el = video;
    }
    const get = PM.assets.get;
    const asset = { id: 'native-ready-media', kind, el, w: 96, h: 64, dur: 2, url, storageKey: 'native-ready-fixture' };
    PM.assets.get = (id: string) => id === asset.id ? asset : get.call(PM.assets, id);
    const project = PM.mkProject({ name: 'Native ready media', w: 96, h: 64, fps: 4, dur: 1, bg: '#21344a' });
    const layer = PM.mkLayer(kind, { from: 0, dur: 1,
      d: { asset: asset.id, w: 83, h: 55, fit: 'cover', speed: 1, trim: 0, embeddedAudio: false },
      p: { 'position.x': 48, 'position.y': 32 } }, project);
    project.layers = [layer]; PM.proj = project; PM.touch();
    PM.animate(layer, 'rotation', [{ t: 0, v: -8 }, { t: 1, v: 19 }]);
    await PM.prepareFrame(0); PM.GL.dropTextures();
    if (typeof PM.GL.createOpaqueReadback !== 'function') throw new Error('Opaque readback factory missing');
    const probe = PM.GL.createOpaqueReadback(96, 64), supported = !!probe; probe?.dispose();
    (window as any).__readyNativeMedia = { asset, seeks: 0, begins: [] as number[] };
    const seeking = () => { (window as any).__readyNativeMedia.seeks++; };
    if (el instanceof HTMLVideoElement) el.addEventListener('seeking', seeking);
    (window as any).__disposeReadyNativeMedia = () => {
      PM.preparedVideoFrames = null; PM.assets.get = get;
      if (el instanceof HTMLVideoElement) { el.removeEventListener('seeking', seeking); el.pause(); el.removeAttribute('src'); el.load(); }
      else el.close();
      URL.revokeObjectURL(url);
    };
    return { supported };
  }, { kind, encoded });
}

for (const kind of ['image', 'video'] as const) test(`native ready ${kind} preserves raw encoder bytes and decoded frames with asynchronous capture`, async ({ session }) => {
  await session.openEditor();
  const { supported } = await readyNativeMedia(session, kind);
  try {
    for (const mblur of [false, true, 'default'] as const) {
      let referenceRaw: string | undefined, referenceDecoded: string | undefined;
      for (const optimized of [false, true]) {
        const output = path.join(session.userData, `ready-${kind}-${mblur}-${optimized}.mp4`);
        await session.app.evaluate(({ ipcMain, dialog }, output) => {
          dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
          const ipc = ipcMain as any;
          const originals = (globalThis as any).__readyNativeHandlers ||= new Map(
            ['render:write', 'render:finish'].map(channel => [channel, ipc._invokeHandlers.get(channel)]));
          const hash = process.getBuiltinModule('node:crypto').createHash('sha256');
          const state = { active: 0, maxActive: 0, bytes: 0, writes: 0, rawHash: '' };
          (globalThis as any).__readyNativeWrites = state;
          for (const [channel, original] of originals) {
            ipcMain.removeHandler(channel);
            ipcMain.handle(channel, async (...args: any[]) => {
              state.maxActive = Math.max(state.maxActive, ++state.active);
              try {
                if (channel === 'render:write') {
                  if (args[1].repeat !== undefined) throw new Error('Animated media unexpectedly used a static repeat');
                  hash.update(args[1].data); state.bytes += args[1].data.length; state.writes++;
                }
                const result = await original(...args);
                if (channel === 'render:finish') state.rawHash = hash.digest('hex');
                return result;
              } finally { state.active--; }
            });
          }
        }, output);
        const result = await session.page.evaluate(async ({ optimized, mblur }) => {
          const PM = (window as any).PM, factory = PM.GL.createOpaqueReadback;
          if (!PM.proj.layers.every((layer: any) => layer.mblur === false)) throw new Error('Expected literal motion-blur flags');
          await PM.prepareFrame(0); PM.GL.dropTextures();
          const state = { sessions: 0, begins: 0, reads: 0, disposed: 0, sampling: [] as Array<{ motionBlur: boolean; samples: number }> }, targets = new Set<Uint8Array>();
          PM.GL.createOpaqueReadback = optimized ? (...args: any[]) => {
            const readback = factory(...args); if (!readback) return readback;
            state.sessions++;
            return {
              begin(...args: any[]) {
                const ticket = readback.begin(...args); state.begins++;
                state.sampling.push({ motionBlur: args[1].mblur, samples: args[1].mbSamples });
                return { readInto(target: Uint8Array, cancelled: () => boolean) {
                  targets.add(target); state.reads++; return ticket.readInto(target, cancelled);
                }, dispose() { ticket.dispose(); } };
              }, dispose() { state.disposed++; readback.dispose(); },
            };
          } : undefined;
          const before = PM.serialize(), time = PM.time, quality = PM.quality;
          try {
            const result = await PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4,
              range: 'all', quality: 'draft', audio: false, ...(mblur === 'default' ? {} : { mblur }) });
            return { result, ...state, targets: targets.size,
              preserved: before === PM.serialize() && time === PM.time && quality === PM.quality };
          } finally { PM.GL.createOpaqueReadback = factory; }
        }, { optimized, mblur });
        expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true);
        expect(result).toMatchObject(optimized && supported
          ? { sessions: 1, begins: 4, reads: 4, disposed: 1, targets: 1 }
          : { sessions: 0, begins: 0, reads: 0, disposed: 0, targets: 0 });
        expect(result.sampling).toEqual(optimized && supported
          ? Array.from({ length: 4 }, () => ({ motionBlur: mblur !== false, samples: mblur === false ? 1 : 16 })) : []);
        const main = await session.app.evaluate(() => (globalThis as any).__readyNativeWrites);
        expect(main).toMatchObject({ active: 0, maxActive: 1, bytes: 96 * 64 * 4 * 4, writes: 4 });
        const decoded = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
          ['-v', 'error', '-i', output, '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'], { encoding: 'utf8' });
        const frames = decoded.split('\n').filter(line => line && !line.startsWith('#'));
        expect(frames).toHaveLength(4); expect(new Set(frames.map(line => line.split(',').at(-1))).size).toBeGreaterThan(1);
        if (referenceRaw === undefined) { referenceRaw = main.rawHash; referenceDecoded = decoded; }
        else { expect(main.rawHash).toBe(referenceRaw); expect(decoded).toBe(referenceDecoded); }
      }
    }
  } finally { await session.page.evaluate(() => (window as any).__disposeReadyNativeMedia()); }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const failure of [false, true]) test(`native video waits for an encoder acknowledgment before the next seek when ${failure ? 'a write fails' : 'cancelled'}`, async ({ session }) => {
  await session.openEditor();
  const { supported } = await readyNativeMedia(session, 'video');
  try {
    test.skip(!supported, 'Asynchronous opaque readback unavailable');
    await session.app.evaluate(({ ipcMain, dialog }, { output, failure }) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      const state = { held: false, writes: 0, finishes: 0, cancels: 0 };
      (globalThis as any).__heldNativeVideo = state;
      for (const name of ['write', 'finish', 'cancel']) {
        const channel = `render:${name}`, original = (ipcMain as any)._invokeHandlers.get(channel);
        ipcMain.removeHandler(channel);
        ipcMain.handle(channel, async (...args: any[]) => {
          if (name === 'finish') state.finishes++;
          if (name === 'cancel') state.cancels++;
          if (name !== 'write') return original(...args);
          state.writes++; await original(...args);
          if (state.writes === 1) {
            state.held = true;
            await new Promise<void>(resolve => { (globalThis as any).__releaseNativeVideo = resolve; });
            state.held = false;
            if (failure) throw new Error('Synthetic held encoder write failure');
          }
        });
      }
    }, { output: path.join(session.userData, `held-video-${failure}.mp4`), failure });
    await session.page.evaluate(() => {
      const PM = (window as any).PM, factory = PM.GL.createOpaqueReadback;
      PM.GL.createOpaqueReadback = (...args: any[]) => {
        const readback = factory(...args); if (!readback) return readback;
        return { begin(...args: any[]) {
          (window as any).__readyNativeMedia.begins.push(args[0]); return readback.begin(...args);
        }, dispose() { readback.dispose(); } };
      };
      (window as any).__restoreNativePreparation = () => { PM.GL.createOpaqueReadback = factory; };
      (window as any).__heldNativeResult = null;
      (window as any).__heldNativeJob = PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4,
        range: 'all', quality: 'draft', audio: false, mblur: false }).then((result: any) => {
          (window as any).__heldNativeResult = result; return result;
        }, (error: any) => { (window as any).__heldNativeResult = { unexpectedRejection: String(error) }; });
    });
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__heldNativeVideo.held)).toBe(true);
    const before = await session.page.evaluate(() => {
      const PM = (window as any).PM, media = (window as any).__readyNativeMedia;
      PM.Export.cancel = true;
      return { currentTime: media.asset.el.currentTime, seeks: media.seeks, begins: media.begins.slice() };
    });
    // Give an incorrectly speculative preparation an event-loop turn while the
    // first real encoder ACK is held. Cancellation must not resolve it early.
    await session.page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    expect(await session.page.evaluate(() => {
      const media = (window as any).__readyNativeMedia;
      return { currentTime: media.asset.el.currentTime, seeks: media.seeks, begins: media.begins.slice(), result: (window as any).__heldNativeResult };
    })).toEqual({ ...before, result: null });
    expect(before.currentTime).toBe(0);
    expect(before.begins).toEqual([0]);
    await session.app.evaluate(() => (globalThis as any).__releaseNativeVideo());
    const result = await session.page.evaluate(() => (window as any).__heldNativeJob);
    if (failure) expect(result.error).toContain('Synthetic held encoder write failure');
    else expect(result).toEqual({ cancelled: true });
    expect(await session.app.evaluate(() => (globalThis as any).__heldNativeVideo)).toEqual({ held: false, writes: 1, finishes: 0, cancels: 1 });
    expect(await session.page.evaluate(() => (window as any).PM.Export.busy)).toBe(false);
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseNativeVideo?.());
    await session.page.evaluate(async () => {
      await (window as any).__heldNativeJob;
      (window as any).__restoreNativePreparation?.(); (window as any).__disposeReadyNativeMedia();
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('native image export recaptures a queued timestamp after its bitmap is replaced', async ({ session }) => {
  await session.openEditor();
  const { supported } = await readyNativeMedia(session, 'image');
  try {
    test.skip(!supported, 'Asynchronous opaque readback unavailable');
    const expected = await session.page.evaluate(async () => {
      const PM = (window as any).PM, media = (window as any).__readyNativeMedia, asset = media.asset;
      const original = asset.el as ImageBitmap;
      const canvas = document.createElement('canvas'); canvas.width = 96; canvas.height = 64;
      const ctx = canvas.getContext('2d')!; ctx.fillStyle = '#ffc531'; ctx.fillRect(0, 0, 96, 64);
      ctx.fillStyle = '#3758e9'; ctx.fillRect(7, 23, 61, 36);
      const replacement = await createImageBitmap(canvas);
      const replace = (bitmap: ImageBitmap) => {
        asset.el = bitmap;
        PM.GL.dropTextures(`a:${asset.id}`); PM.bus.emit('assets');
      };
      const reference: number[] = [];
      try {
        for (let frame = 0; frame < 4; frame++) {
          if (frame === 1) replace(replacement);
          await PM.prepareFrame(frame / 4);
          reference.push(...PM.GL.renderToPixels(frame / 4, 96, 64,
            { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 }));
        }
      } finally { replace(original); await PM.prepareFrame(0); }
      (window as any).__replaceQueuedNativeImage = () => replace(replacement);
      (window as any).__disposeQueuedNativeImage = () => { replace(original); replacement.close(); };
      return reference;
    });
    await session.app.evaluate(({ ipcMain, dialog }, output) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      const state = { held: false, chunks: [] as number[][] };
      (globalThis as any).__queuedNativeImage = state;
      const original = (ipcMain as any)._invokeHandlers.get('render:write');
      ipcMain.removeHandler('render:write');
      ipcMain.handle('render:write', async (...args: any[]) => {
        state.chunks.push(Array.from(args[1].data)); await original(...args);
        if (state.chunks.length === 1) {
          state.held = true;
          await new Promise<void>(resolve => { (globalThis as any).__releaseQueuedNativeImage = resolve; });
          state.held = false;
        }
      });
    }, path.join(session.userData, 'replaced-native-image.mp4'));
    await session.page.evaluate(() => {
      const PM = (window as any).PM, factory = PM.GL.createOpaqueReadback;
      const state = { begins: [] as number[], target: null as Uint8Array | null, first: null as Uint8Array | null };
      (window as any).__queuedNativeImagePixels = state;
      PM.GL.createOpaqueReadback = (...args: any[]) => {
        const readback = factory(...args); if (!readback) return readback;
        return {
          begin(...args: any[]) {
            state.begins.push(args[0]); const ticket = readback.begin(...args);
            return { async readInto(target: Uint8Array, cancelled: () => boolean) {
              const result = await ticket.readInto(target, cancelled);
              if (!state.first) { state.target = target; state.first = target.slice(); }
              return result;
            }, dispose() { ticket.dispose(); } };
          }, dispose() { readback.dispose(); },
        };
      };
      (window as any).__restoreQueuedNativeImage = () => { PM.GL.createOpaqueReadback = factory; };
      (window as any).__queuedNativeImageJob = PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4,
        range: 'all', quality: 'draft', audio: false, mblur: false });
    });
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__queuedNativeImage.held)).toBe(true);
    await expect.poll(() => session.page.evaluate(() => (window as any).__queuedNativeImagePixels.begins)).toEqual([0, .25]);
    expect(await session.page.evaluate(() => {
      const state = (window as any).__queuedNativeImagePixels;
      (window as any).__replaceQueuedNativeImage();
      return state.first.every((byte: number, index: number) => byte === state.target[index]);
    })).toBe(true);
    await session.app.evaluate(() => (globalThis as any).__releaseQueuedNativeImage());
    expect(await session.page.evaluate(() => (window as any).__queuedNativeImageJob)).toEqual({ cancelled: false });
    expect(await session.app.evaluate(() => (globalThis as any).__queuedNativeImage.chunks.flat())).toEqual(expected);
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseQueuedNativeImage?.());
    await session.page.evaluate(async () => {
      await (window as any).__queuedNativeImageJob;
      (window as any).__restoreQueuedNativeImage?.(); (window as any).__disposeQueuedNativeImage?.();
      (window as any).__disposeReadyNativeMedia();
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
