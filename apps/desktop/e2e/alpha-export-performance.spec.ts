import { execFileSync } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from 'playwright';
import { expect, test, repoRoot } from './helpers/app';

async function supportsIntegerReadback(page: Page, depth: number): Promise<boolean> {
  return page.evaluate(depth => {
    const PM = (window as any).PM, gl = PM.GL.gl;
    const factory = PM.GL[depth === 32 ? 'createFloatReadback' : 'createHalfReadback'];
    if (typeof factory !== 'function') throw new Error('The production alpha readback factory is unavailable');
    const width = PM.proj.w, height = PM.proj.h, read = gl.readPixels;
    let integerReads = 0, floatReads = 0, session: any;
    gl.readPixels = function (...args: any[]) {
      const result = read.apply(gl, args);
      // Ignore the 257-pixel compatibility probe. A session can exist while
      // its lazy gate or actual transfer still chooses the original fallback.
      if (args[2] === width && args[4] === gl.RGBA_INTEGER) integerReads++;
      if (args[2] === width && args[5] === gl.FLOAT) floatReads++;
      return result;
    };
    try {
      session = factory(width, height);
      if (!session) return false;
      const pixels = session.capture(PM.time, { transparent: true, mblur: false, mbSamples: 20, shutter: PM.proj.shutter || .5 });
      return !!pixels && integerReads > 0 && floatReads === 0;
    } finally {
      try { session?.dispose(); } finally { gl.readPixels = read; }
    }
  }, depth);
}

for (const depth of [16, 32]) for (const { groupedBlur, still, width, height } of [
  { groupedBlur: false, still: false, width: 1920, height: 1080 },
  { groupedBlur: true, still: false, width: 1920, height: 1080 },
  { groupedBlur: false, still: true, width: 1920, height: 1080 },
  { groupedBlur: false, still: false, width: 1280, height: 720 },
  { groupedBlur: false, still: false, width: 640, height: 360 },
]) test(`preserves every transparent ${still ? 'still PNG' : 'ProRes'} pixel at ${width}×${height} with ${depth}-bit readback${groupedBlur ? ', groups and motion blur' : ''}`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const frames = still ? 1 : groupedBlur ? 6 : 12;
  const independentPNG = await session.page.evaluate(async ({ frames, groupedBlur, still, depth, width, height }) => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Transparent delivery', w: width, h: height, dur: 1, fps: frames });
    project.colorDepth = depth;
    project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const scale = width / 1920;
    const shape = PM.mkLayer('shape', { d: { w: 1080 * scale, h: 720 * scale, radius: 17 * scale, color: '#a34bf7' },
      p: { 'position.x': 960 * scale, 'position.y': 540 * scale, opacity: 53 } }, project);
    const overlap = PM.mkLayer('shape', { d: { w: 330 * scale, h: 440 * scale, radius: 29 * scale, color: '#39d38a' },
      p: { 'position.x': 1140 * scale, 'position.y': 420 * scale, opacity: 31, rotation: 13 } }, project);
    project.layers = [shape, overlap]; PM.proj = project; PM.touch();
    PM.animate(shape, 'rotation', [{ t: 0, v: 0 }, { t: 1, v: 50 }]);
    if (groupedBlur) {
      PM.Edit.apply({ type: 'group_layers', targets: [shape.id, overlap.id], name: 'Alpha group' }, { origin: 'agent' });
      const group = PM.proj.layers.find((layer: any) => layer.type === 'group');
      group.mblur = true; shape.mblur = true; overlap.mblur = true;
      PM.proj.shutter = -.75;
      PM.animate(group, 'rotation', [{ t: 0, v: -7 }, { t: 1, v: 19 }]);
    }
    if (!still) return null;
    // Independent original alpha route: FLOAT readback/CPU quantization in GL,
    // the established integer unpremultiply, then the actual Canvas PNG encoder.
    const pixels = PM.GL.renderToPixels(PM.time, width, height, { transparent: true, mblur: false, mbSamples: 20, shutter: .5 });
    const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
    try {
      const context = canvas.getContext('2d')!, image = context.createImageData(width, height);
      for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
        const source = ((height - 1 - y) * width + x) * 4, target = (y * width + x) * 4, alpha = pixels[source + 3];
        for (let channel = 0; channel < 3; channel++) image.data[target + channel] = alpha === 255 ? pixels[source + channel]
          : alpha === 0 ? 0 : Math.min(255, (pixels[source + channel] * 255 / alpha) | 0);
        image.data[target + 3] = alpha;
      }
      context.putImageData(image, 0, 0);
      const blob = await new Promise<Blob>((resolve, reject) => canvas.toBlob(value => value ? resolve(value) : reject(new Error('Reference PNG encoding failed'))));
      return Array.from(new Uint8Array(await blob.arrayBuffer()));
    } finally { canvas.width = 0; canvas.height = 0; }
  }, { frames, groupedBlur, still, depth, width, height });
  const optimized = await supportsIntegerReadback(session.page, depth);

  const outputs: string[] = [];
  for (const enabled of [false, true]) {
    const output = path.join(session.userData, `alpha-${enabled ? 'integer' : 'reference'}.${still ? 'png' : 'mov'}`);
    outputs.push(output);
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, output);
    const metrics = await session.page.evaluate(async ({ enabled, frames, groupedBlur, still, depth, width, height }) => {
      const PM = (window as any).PM;
      PM.agentFrameCapture = true;
      const name = depth === 32 ? 'createFloatReadback' : 'createHalfReadback', factory = PM.GL[name], asyncFactory = PM.GL.createAlphaReadback;
      if (typeof factory !== 'function') throw new Error('The production alpha readback factory is unavailable');
      const create = CanvasRenderingContext2D.prototype.createImageData;
      let images = 0, sessions = 0, captures = 0, disposals = 0, buffers = 0;
      const surfaces = new Set<HTMLCanvasElement>(), owned = new WeakSet<ArrayBuffer>();
      CanvasRenderingContext2D.prototype.createImageData = function (...args: any[]) {
        if (args[0] === width && args[1] === height) { images++; surfaces.add(this.canvas); }
        return (create as any).apply(this, args);
      };
      PM.GL[name] = enabled ? (width: number, height: number) => {
        const session = factory(width, height);
        if (!session) return null;
        sessions++;
        return {
          capture(time: number, options: any) {
            captures++;
            const pixels = session.capture(time, options);
            if (pixels) {
              if (pixels.length !== width * height * 4) throw new Error('Incorrect alpha capture size');
              if (!owned.has(pixels.buffer)) { owned.add(pixels.buffer); buffers++; }
            }
            return pixels;
          },
          dispose() { disposals++; session.dispose(); },
        };
      } : undefined;
      PM.GL.createAlphaReadback = undefined;
      try {
        const result = await PM.Export.run({ format: still ? 'still' : 'prores', alpha: true, scale: 1, fps: frames,
          range: 'all', audio: false, mblur: groupedBlur });
        return { result, images, surfaces: surfaces.size, sessions, captures, disposals, buffers,
          released: [...surfaces].every(canvas => canvas.width === 0 && canvas.height === 0) };
      } finally {
        PM.GL[name] = factory;
        PM.GL.createAlphaReadback = asyncFactory;
        CanvasRenderingContext2D.prototype.createImageData = create;
      }
    }, { enabled, frames, groupedBlur, still, depth, width, height });
    expect(metrics.result).toEqual({ cancelled: false });
    expect(metrics.images).toBe(1); expect(metrics.surfaces).toBe(1); expect(metrics.released).toBe(true);
    if (!enabled || optimized) {
      expect(metrics.sessions).toBe(enabled ? 1 : 0);
      expect(metrics.captures).toBe(enabled ? frames : 0);
      expect(metrics.disposals).toBe(enabled ? 1 : 0);
      expect(metrics.buffers).toBe(enabled ? 1 : 0);
    } else {
      // Output parity below remains mandatory when this GPU uses fallback.
      expect(metrics.sessions).toBeLessThanOrEqual(1);
      expect(metrics.disposals).toBe(metrics.sessions);
      expect(metrics.captures).toBe(metrics.sessions ? frames : 0);
      expect(metrics.buffers).toBeLessThanOrEqual(metrics.captures);
      if (metrics.captures) expect(metrics.buffers).toBeGreaterThan(0);
    }
  }

  // Hash every decoded RGBA byte at full resolution; each record also carries
  // its DTS, PTS, duration and byte count, so no frame or timestamp can drift.
  const decoded = outputs.map(output => execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
    ['-v', 'error', '-i', output, '-map', '0:v:0', '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'],
    { maxBuffer: 1024 * 1024 }).toString());
  expect(decoded[1]).toBe(decoded[0]);
  if (independentPNG) {
    const reference = path.join(session.userData, 'alpha-independent-reference.png');
    await writeFile(reference, new Uint8Array(independentPNG));
    const original = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
      ['-v', 'error', '-i', reference, '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'],
      { maxBuffer: 1024 * 1024 }).toString();
    expect(decoded[1]).toBe(original);
  }
  const records = decoded[0]!.split('\n').filter(line => line && !line.startsWith('#'));
  expect(records).toHaveLength(frames);
  if (frames > 1) expect(new Set(records.map(line => line.split(',').at(-1)!.trim())).size).toBeGreaterThan(1);
  for (const line of records) expect(Number(line.split(',')[4])).toBe(width * height * 4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const depth of [16, 32]) test(`rejects GPU loss during a large ${depth}-bit integer alpha read, revalidates after restoration, and retries exactly`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const output = path.join(session.userData, 'alpha-recovered.mov');
  await session.app.evaluate(({ ipcMain, dialog }, filePath) => {
    const state = { writes: 0, finishes: 0, cancels: 0 };
    (globalThis as any).__halfCaptureRecovery = state;
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    // Preserve the real trusted-owner handlers, encoder and cleanup path.
    for (const name of ['write', 'finish', 'cancel']) {
      const channel = `render:${name}`, original = (ipcMain as any)._invokeHandlers.get(channel);
      ipcMain.removeHandler(channel);
      ipcMain.handle(channel, async (...args: any[]) => {
        if (name === 'write') state.writes++;
        if (name === 'finish') state.finishes++;
        if (name === 'cancel') state.cancels++;
        return await original(...args);
      });
    }
  }, output);
  const supported = await session.page.evaluate(depth => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Alpha GPU recovery', w: 1920, h: 1080, fps: 2, dur: 1 });
    project.colorDepth = depth; project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const shape = PM.mkLayer('shape', { d: { w: 1080, h: 720, radius: 31, color: '#dd7355' },
      p: { 'position.x': 960, 'position.y': 540, opacity: 37 } }, project);
    project.layers = [shape]; PM.proj = project; PM.touch();
    PM.animate(shape, 'rotation', [{ t: 0, v: 0 }, { t: 1, v: 50 }]);
    (window as any).__halfGpuLoss = PM.GL.gl.getExtension('WEBGL_lose_context');
    return !!(window as any).__halfGpuLoss;
  }, depth);
  test.skip(!supported, 'GPU reset extension unavailable');
  test.skip(!await supportsIntegerReadback(session.page, depth), 'Validated integer alpha readback unavailable on this GPU');
  const failed = await session.page.evaluate(async depth => {
    const PM = (window as any).PM, gl = PM.GL.gl, read = gl.readPixels;
    const name = depth === 32 ? 'createFloatReadback' : 'createHalfReadback', factory = PM.GL[name], asyncFactory = PM.GL.createAlphaReadback;
    let integerReads = 0, captures = 0, disposed = 0;
    gl.readPixels = function (...args: any[]) {
      const result = read.apply(gl, args);
      // The exhaustive validation uses width 257. Interrupt only the actual
      // full frame read (which also contains its private guard row).
      if (args[2] === 1920 && args[4] === gl.RGBA_INTEGER) {
        integerReads++; (window as any).__halfGpuLoss.loseContext();
      }
      return result;
    };
    PM.GL[name] = (...args: any[]) => {
      const session = factory(...args);
      if (!session) throw new Error('Validated alpha readback was not created');
      return {
        capture(...args: any[]) { captures++; return session.capture(...args); },
        dispose() { disposed++; session.dispose(); },
      };
    };
    PM.GL.createAlphaReadback = undefined;
    try {
      const result = await PM.Export.run({ format: 'prores', alpha: true, scale: 1, fps: 2,
        range: 'all', audio: false, mblur: false });
      return { result, integerReads, captures, disposed, busy: PM.Export.busy };
    } finally { gl.readPixels = read; PM.GL[name] = factory; PM.GL.createAlphaReadback = asyncFactory; }
  }, depth);
  expect(failed).toEqual({ result: { error: 'The GPU could not capture the export frame' }, integerReads: 1, captures: 1, disposed: 1, busy: false });
  expect(await session.app.evaluate(() => (globalThis as any).__halfCaptureRecovery)).toEqual({ writes: 0, finishes: 0, cancels: 1 });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.GL.contextLost)).toBe(true);
  await session.page.evaluate(() => (window as any).__halfGpuLoss.restoreContext());
  await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.GL.gl && !(window as any).PM.GL.contextLost)).toBe(true);

  const retry = await session.page.evaluate(async () => {
    const PM = (window as any).PM, gl = PM.GL.gl, read = gl.readPixels, asyncFactory = PM.GL.createAlphaReadback;
    let integerReads = 0, validationReads = 0;
    gl.readPixels = function (...args: any[]) {
      const result = read.apply(gl, args);
      if (args[4] === gl.RGBA_INTEGER) {
        if (args[2] === 1920) integerReads++;
        else if (args[2] === 257) validationReads++;
      }
      return result;
    };
    PM.GL.createAlphaReadback = undefined;
    try {
      const result = await PM.Export.run({ format: 'prores', alpha: true, scale: 1, fps: 2,
        range: 'all', audio: false, mblur: false });
      return { result, integerReads, validationReads };
    } finally { gl.readPixels = read; PM.GL.createAlphaReadback = asyncFactory; }
  });
  expect(retry).toEqual({ result: { cancelled: false }, integerReads: 2, validationReads: 1 });
  expect(await session.app.evaluate(() => (globalThis as any).__halfCaptureRecovery)).toEqual({ writes: 4, finishes: 1, cancels: 1 });

  const reference = path.join(session.userData, 'alpha-recovery-reference.mov');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, reference);
  expect(await session.page.evaluate(async depth => {
    const PM = (window as any).PM, name = depth === 32 ? 'createFloatReadback' : 'createHalfReadback', factory = PM.GL[name], asyncFactory = PM.GL.createAlphaReadback;
    PM.GL[name] = undefined;
    PM.GL.createAlphaReadback = undefined;
    try { return await PM.Export.run({ format: 'prores', alpha: true, scale: 1, fps: 2,
      range: 'all', audio: false, mblur: false }); }
    finally { PM.GL[name] = factory; PM.GL.createAlphaReadback = asyncFactory; }
  }, depth)).toEqual({ cancelled: false });
  const decoded = [output, reference].map(filePath => execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
    ['-v', 'error', '-i', filePath, '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'],
    { maxBuffer: 1024 * 1024 }).toString());
  expect(decoded[0]).toBe(decoded[1]);
  expect(decoded[0]!.split('\n').filter(line => line && !line.startsWith('#'))).toHaveLength(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const depth of [16, 32]) test(`asynchronous ${depth}-bit alpha readback matches original FLOAT quantization on odd rows`, async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(depth => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.quality = .5; PM.AutomaticPreview?.cancel();
    const project = PM.mkProject({ name: 'Odd alpha readback', w: 129, h: 97, fps: 4, dur: 1 });
    project.colorDepth = depth; project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const group = PM.mkLayer('group', { p: { opacity: 73, rotation: 13 } }, project);
    const shape = PM.mkLayer('shape', { d: { w: 87, h: 63, radius: 17, color: '#a34bf790' },
      p: { 'position.x': 59, 'position.y': 43, opacity: 53 } }, project);
    const overlap = PM.mkLayer('shape', { d: { shape: 'ellipse', w: 47, h: 51, color: '#39d38acf' },
      p: { 'position.x': 81, 'position.y': 57, opacity: 31 } }, project);
    shape.group = group.id; overlap.group = group.id;
    const glow = PM.mkEffect('glow'); glow.p.radius.v = 9; glow.p.intensity.v = 113; group.fx = [glow];
    project.layers = [group, shape, overlap]; PM.proj = project; PM.touch();
    PM.animate(shape, 'rotation', [{ t: 0, v: -11 }, { t: 1, v: 41 }]);
    if (typeof PM.GL.createAlphaReadback !== 'function') throw new Error('The asynchronous production alpha factory is missing');
  }, depth);
  test.skip(!await supportsIntegerReadback(session.page, depth), 'Validated integer alpha readback unavailable on this GPU');
  const metrics = await session.page.evaluate(async () => {
    const PM = (window as any).PM, gl = PM.GL.gl, width = 129, height = 97, visible = width * height * 4;
    const options = { transparent: true, mblur: false, mbSamples: 20, shutter: PM.proj.shutter || .5 };
    const saved = { project: PM.proj, source: PM.serialize(), time: PM.time, quality: PM.quality };
    const bufferData = gl.bufferData, deleteBuffer = gl.deleteBuffer, fenceSync = gl.fenceSync, deleteSync = gl.deleteSync;
    const readPixels = gl.readPixels, getBufferSubData = gl.getBufferSubData;
    const buffers = new Set<WebGLBuffer>(), fences = new Set<WebGLSync>();
    const stats = { pboAllocations: 0, pboDeletes: 0, pboBytes: 0, activeFences: 0, maxFences: 0,
      reads: 0, queued: 0, rawAllocations: 0, mismatches: [] as number[], hashes: [] as string[], partial: [] as number[] };
    const previousPack = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
    const marker = gl.createBuffer(); if (!marker) throw new Error('Could not create state marker');
    let capture: any, ticket: any, raw: Uint8Array | null = null, release: (() => void) | undefined, pendingRead: Promise<boolean> | undefined;
    gl.bufferData = function (...args: any[]) {
      const result = bufferData.apply(gl, args);
      if (args[0] === gl.PIXEL_PACK_BUFFER) {
        const buffer = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING); buffers.add(buffer);
        stats.pboAllocations++; stats.pboBytes += args[1];
      }
      return result;
    };
    gl.deleteBuffer = function (buffer: WebGLBuffer) {
      if (buffers.delete(buffer)) stats.pboDeletes++; return deleteBuffer.call(gl, buffer);
    };
    gl.fenceSync = function (...args: any[]) {
      const fence = fenceSync.apply(gl, args); if (fence) fences.add(fence);
      stats.activeFences = fences.size; stats.maxFences = Math.max(stats.maxFences, fences.size); return fence;
    };
    gl.deleteSync = function (fence: WebGLSync) {
      fences.delete(fence); stats.activeFences = fences.size; return deleteSync.call(gl, fence);
    };
    gl.readPixels = function (...args: any[]) {
      if (args[2] === width && args[4] === gl.RGBA_INTEGER && typeof args[6] === 'number') stats.queued++;
      return readPixels.apply(gl, args);
    };
    gl.getBufferSubData = function (...args: any[]) {
      if (args[0] === gl.PIXEL_PACK_BUFFER) {
        if (args[2] !== raw) throw new Error('The alpha CPU target was replaced'); stats.reads++;
      }
      return getBufferSubData.apply(gl, args);
    };
    try {
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, marker);
      capture = PM.GL.createAlphaReadback(width, height);
      if (!capture) throw new Error('Validated integer alpha capture did not create an asynchronous session');
      if (capture.byteLength !== width * (height + 1) * 4 || gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING) !== marker)
        throw new Error('Incorrect guard size or factory pack-binding restoration');
      for (const time of [0, .25, .5]) {
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
        await PM.prepareFrame(time);
        // Public renderToPixels without either private reader is the original
        // FLOAT read plus JavaScript double-precision clamp/round conversion.
        const reference = PM.GL.renderToPixels(time, width, height, options);
        if (!reference || reference.length !== visible) throw new Error('Invalid original alpha capture');
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, marker);
        ticket = capture.begin(time, options); if (!ticket) throw new Error('Expected an actual deferred alpha ticket');
        if (PM.quality !== .5 || gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING) !== marker) throw new Error('Submission leaked alpha quality or pack state');
        const reads = stats.reads;
        const ack = new Promise<void>(resolve => { release = resolve; });
        const reading = ticket, byteLength = capture.byteLength;
        const copied = pendingRead = (async () => {
          await ack;
          if (!raw) { raw = new Uint8Array(byteLength); stats.rawAllocations++; }
          return reading.readInto(raw, () => false);
        })();
        await Promise.resolve();
        if (stats.reads !== reads || time === 0 && raw !== null) throw new Error('CPU readback started before the preceding ACK');
        release!();
        if (!await copied) throw new Error('Alpha readback unexpectedly cancelled');
        ticket.dispose(); ticket = null;
        if (gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING) !== marker || PM.quality !== .5) throw new Error('Readback leaked pack state or alpha quality');
        let mismatches = 0, partial = 0;
        for (let i = 0; i < visible; i++) if (raw![i] !== reference[i]) mismatches++;
        for (let i = 3; i < visible; i += 4) if (raw![i] > 0 && raw![i] < 255) partial++;
        for (let i = visible; i < raw!.length; i++) {
          if (!raw![i] || raw![i] !== raw![visible + i % 4]) throw new Error('Invalid alpha nonce guard row');
        }
        stats.mismatches.push(mismatches); stats.partial.push(partial);
        stats.hashes.push(Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', raw!.subarray(0, visible))))
          .map(byte => byte.toString(16).padStart(2, '0')).join(''));
      }
      capture.dispose(); capture = null;
      return { ...stats, activeBuffers: buffers.size,
        preserved: saved.project === PM.proj && saved.source === PM.serialize() && saved.time === PM.time && saved.quality === PM.quality };
    } finally {
      release?.(); ticket?.dispose(); capture?.dispose();
      await pendingRead?.catch(() => {});
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, previousPack); deleteBuffer.call(gl, marker);
      gl.bufferData = bufferData; gl.deleteBuffer = deleteBuffer; gl.fenceSync = fenceSync; gl.deleteSync = deleteSync;
      gl.readPixels = readPixels; gl.getBufferSubData = getBufferSubData;
    }
  });
  expect(metrics.preserved).toBe(true); expect(metrics.mismatches).toEqual([0, 0, 0]);
  expect(new Set(metrics.hashes).size).toBe(3); expect(metrics.partial.every(count => count > 100)).toBe(true);
  expect(metrics).toMatchObject({ pboAllocations: 1, pboDeletes: 1, pboBytes: 129 * 98 * 4,
    activeBuffers: 0, activeFences: 0, maxFences: 1, reads: 3, queued: 3, rawAllocations: 1 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
