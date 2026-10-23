import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

const frames = 4;
const ffmpeg = ffmpegBinary;
const formats = [
  { format: 'mp4', alpha: false }, { format: 'prores', alpha: true },
  { format: 'png', alpha: false }, { format: 'png', alpha: true },
  { format: 'webm', alpha: false },
];

async function bitmapProject(session: LaunchedApp, alpha = false) {
  await session.page.evaluate(async alpha => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.AutomaticPreview?.cancel();
    const canvas = document.createElement('canvas'); canvas.width = 128; canvas.height = 96;
    const ctx = canvas.getContext('2d')!, gradient = ctx.createLinearGradient(0, 0, 128, 96);
    gradient.addColorStop(0, 'rgba(240,37,119,.37)'); gradient.addColorStop(1, 'rgba(25,211,169,.81)');
    ctx.fillStyle = gradient; ctx.fillRect(0, 0, 128, 96);
    ctx.fillStyle = 'rgba(51,79,215,.63)'; ctx.fillRect(19, 13, 71, 57);
    const bitmap = await createImageBitmap(canvas), get = PM.assets.get;
    const asset = { id: 'static-image-source', kind: 'image', el: bitmap, w: 128, h: 96, name: 'source.png', type: 'image/png' };
    PM.assets.get = (id: string) => id === asset.id ? asset : get.call(PM.assets, id);
    const project = PM.mkProject({ name: 'Static image', w: 128, h: 96, fps: 4, dur: 1, bg: '#21344a' });
    if (alpha) project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const layer = PM.mkLayer('image', { from: 0, dur: 1, d: { asset: asset.id, w: 109, h: 81, fit: 'cover' },
      p: { 'position.x': 62, 'position.y': 47, rotation: 7, opacity: 73 } }, project);
    project.layers = [layer]; PM.proj = project; PM.touch(); await PM.prepareFrame(0);
    (window as any).__staticImageFixture = { asset, layer, rotation: structuredClone(layer.p.rotation) };
    (window as any).__disposeStaticImage = () => {
      PM.assets.get = get; bitmap.close(); PM.GL.dropTextures(`a:${asset.id}`);
    };
  }, alpha);
}

function decodedFrames(input: string, png = false) {
  return execFileSync(ffmpeg, ['-v', 'error', ...(png ? ['-framerate', '4'] : []), '-i', input,
    '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'], { encoding: 'utf8' });
}

for (const { format, alpha } of formats) test(`retains static bitmap ${format} pictures with exact ${alpha ? 'transparent' : 'opaque'} output`, async ({ session }) => {
  await session.openEditor(); await bitmapProject(session, alpha);
  await session.app.evaluate(({ ipcMain }) => {
    const original = (ipcMain as any)._invokeHandlers.get('render:write');
    ipcMain.removeHandler('render:write');
    ipcMain.handle('render:write', async (...args: any[]) => {
      const state = (globalThis as any).__staticImageWrites, request = args[1];
      state.maxActive = Math.max(state.maxActive, ++state.active);
      try {
        if (request.repeat !== undefined) {
          state.repeats.push(request.repeat);
          for (let i = 0; i < request.repeat; i++) state.hash.update(state.last);
        } else { state.hash.update(request.data); state.last = Buffer.from(request.data); state.writes++; }
        return await original(...args);
      } finally { state.active--; }
    });
  });
  let rawReference: string | undefined, decodedReference: string | undefined, pngReference: string[] | undefined;
  try {
    for (const retained of [false, true]) {
      const folder = path.join(session.userData, retained ? 'retained' : 'per-frame'); await mkdir(folder);
      const output = path.join(folder, `output.${format === 'prores' ? 'mov' : format}`);
      await session.app.evaluate(({ dialog }, { folder, output }) => {
        dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
        dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
        (globalThis as any).__staticImageWrites = { active: 0, maxActive: 0, writes: 0, repeats: [], last: null,
          hash: process.getBuiltinModule('node:crypto').createHash('sha256') };
      }, { folder, output });
      const result = await session.page.evaluate(async ({ retained, format, alpha }) => {
        const PM = (window as any).PM, fixture = (window as any).__staticImageFixture;
        // Identical values keep the rendered picture constant while two keys
        // deliberately retain the established per-frame baseline.
        fixture.layer.p.rotation = structuredClone(fixture.rotation);
        if (!retained) PM.animate(fixture.layer, 'rotation', [{ t: 0, v: 7 }, { t: 1, v: 7 }]);
        PM.touch(); await PM.prepareFrame(0); PM.GL.dropTextures();
        const pixels = PM.GL.renderToPixels, render = PM.GL.render, encode = HTMLCanvasElement.prototype.toBlob;
        const factories = ['createOpaqueReadback', 'createHalfReadback', 'createFloatReadback'].map(name => [name, PM.GL[name]] as const);
        const state = { captures: 0, presents: 0, encodes: 0, sessions: 0, disposed: 0 };
        PM.GL.renderToPixels = (...args: any[]) => { state.captures++; return pixels(...args); };
        PM.GL.render = (...args: any[]) => { state.presents++; return render(...args); };
        for (const [name, factory] of factories) if (factory) PM.GL[name] = (...args: any[]) => {
          const capture = factory(...args); if (!capture) return capture; state.sessions++;
          return name === 'createOpaqueReadback' ? {
            begin(...args: any[]) { state.captures++; return capture.begin(...args); },
            dispose() { state.disposed++; capture.dispose(); },
          } : {
            capture(...args: any[]) {
              const before = state.captures, result = capture.capture(...args);
              if (before === state.captures) state.captures++; return result;
            }, dispose() { state.disposed++; capture.dispose(); },
          };
        };
        HTMLCanvasElement.prototype.toBlob = function (...args: any[]) { state.encodes++; return (encode as any).apply(this, args); };
        const before = PM.serialize(), time = PM.time, quality = PM.quality;
        try {
          const result = await PM.Export.run({ format, alpha, scale: 1, fps: 4, range: 'all', quality: 'draft', audio: false, mblur: false });
          return { result, ...state, preserved: before === PM.serialize() && time === PM.time && quality === PM.quality };
        } finally {
          PM.GL.renderToPixels = pixels; PM.GL.render = render; HTMLCanvasElement.prototype.toBlob = encode;
          for (const [name, factory] of factories) PM.GL[name] = factory;
        }
      }, { retained, format, alpha });
      expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true);
      expect(result.disposed).toBe(result.sessions);
      if (format === 'webm') expect(result.presents).toBe(retained ? 1 : frames);
      else expect(result.captures).toBe(retained ? 1 : frames);
      if (format === 'png') expect(result.encodes).toBe(retained ? 1 : frames);
      if (format === 'mp4' || format === 'prores') {
        const main = await session.app.evaluate(() => {
          const state = (globalThis as any).__staticImageWrites;
          return { active: state.active, maxActive: state.maxActive, writes: state.writes, repeats: state.repeats, hash: state.hash.digest('hex') };
        });
        expect(main).toMatchObject({ active: 0, maxActive: 1, writes: retained ? 1 : frames, repeats: retained ? [1, 1, 1] : [] });
        if (rawReference === undefined) rawReference = main.hash; else expect(main.hash).toBe(rawReference);
      }
      const input = format === 'png' ? path.join(folder, 'Static image_%05d.png') : output;
      if (!retained) {
        const first = execFileSync(ffmpeg, ['-v', 'error', '-i', input, '-frames:v', '1',
          '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
        expect(first.length).toBe(128 * 96 * 4);
        const colors = new Set<number>();
        for (let at = 0; at < first.length; at += 4) colors.add(first.readUInt32LE(at));
        expect(colors.size).toBeGreaterThan(32);
      }
      const decoded = decodedFrames(input, format === 'png');
      expect(decoded.split('\n').filter(line => line && !line.startsWith('#'))).toHaveLength(frames);
      if (decodedReference === undefined) decodedReference = decoded; else expect(decoded).toBe(decodedReference);
      if (format === 'png') {
        const hashes = await Promise.all(Array.from({ length: frames }, async (_, i) => createHash('sha256')
          .update(await readFile(path.join(folder, `Static image_${String(i).padStart(5, '0')}.png`))).digest('hex')));
        if (pngReference === undefined) pngReference = hashes; else expect(hashes).toEqual(pngReference);
      }
    }
  } finally { await session.page.evaluate(() => (window as any).__disposeStaticImage()); }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const format of ['mp4', 'png']) test(`static image ${format} refreshes retained pixels after replacement while a write acknowledgment is held`, async ({ session }) => {
  await session.openEditor(); await bitmapProject(session);
  const folder = path.join(session.userData, 'replacement'); await mkdir(folder);
  const expected = await session.page.evaluate(async format => {
    const PM = (window as any).PM, fixture = (window as any).__staticImageFixture, asset = fixture.asset, original = asset.el;
    const canvas = document.createElement('canvas'); canvas.width = 128; canvas.height = 96;
    const ctx = canvas.getContext('2d')!; ctx.fillStyle = '#f2bc31'; ctx.fillRect(0, 0, 128, 96);
    ctx.fillStyle = '#3658dc'; ctx.fillRect(11, 27, 79, 61);
    const replacement = await createImageBitmap(canvas);
    const replace = (el: ImageBitmap) => { asset.el = el; PM.GL.dropTextures(`a:${asset.id}`); PM.bus.emit('assets'); };
    const reference: number[] = [];
    try {
      for (let frame = 0; frame < 4; frame++) {
        if (frame === 1) replace(replacement);
        await PM.prepareFrame(frame / 4);
        if (format === 'mp4') reference.push(...PM.GL.renderToPixels(frame / 4, 128, 96,
          { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 }));
        else {
          const rendered = PM.renderFrameTo(frame / 4, 128, 96, { mblur: false });
          reference.push(...rendered.getContext('2d').getImageData(0, 0, 128, 96).data);
          rendered.width = 0; rendered.height = 0;
        }
      }
    } finally { replace(original); await PM.prepareFrame(0); }
    (window as any).__replaceStaticImage = () => replace(replacement);
    (window as any).__disposeReplacementImage = () => { replace(original); replacement.close(); };
    return reference;
  }, format);
  try {
    await session.app.evaluate(({ ipcMain, dialog }, { folder, format }) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: `${folder}/replacement.mp4` });
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
      const channel = format === 'mp4' ? 'render:write' : 'file:save';
      const original = (ipcMain as any)._invokeHandlers.get(channel), state = { held: false, writes: 0, chunks: [] as number[][] };
      (globalThis as any).__heldStaticImage = state;
      ipcMain.removeHandler(channel); ipcMain.handle(channel, async (...args: any[]) => {
        const request = args[1]; state.writes++;
        if (format === 'mp4') {
          if (request.repeat !== undefined) for (let i = 0; i < request.repeat; i++) state.chunks.push(state.chunks.at(-1)!);
          else state.chunks.push(Array.from(request.data));
        }
        const result = await original(...args);
        if (state.writes === 1) {
          state.held = true; await new Promise<void>(resolve => { (globalThis as any).__releaseStaticImage = resolve; }); state.held = false;
        }
        return result;
      });
    }, { folder, format });
    await session.page.evaluate(format => {
      const PM = (window as any).PM;
      (window as any).__staticImageResult = null;
      (window as any).__staticImageJob = PM.Export.run({ format, alpha: false, scale: 1, fps: 4,
        range: 'all', quality: 'draft', audio: false, mblur: false }).then((result: any) => {
          (window as any).__staticImageResult = result; return result;
        });
    }, format);
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__heldStaticImage.held)).toBe(true);
    await session.page.evaluate(() => (window as any).__replaceStaticImage());
    expect(await session.page.evaluate(() => (window as any).__staticImageResult)).toBeNull();
    await session.app.evaluate(() => (globalThis as any).__releaseStaticImage());
    expect(await session.page.evaluate(() => (window as any).__staticImageJob)).toEqual({ cancelled: false });
    if (format === 'mp4') expect(await session.app.evaluate(() => (globalThis as any).__heldStaticImage.chunks.flat())).toEqual(expected);
    else {
      const decoded = execFileSync(ffmpeg, ['-v', 'error', '-framerate', '4', '-i', path.join(folder, 'Static image_%05d.png'),
        '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
      expect(decoded).toEqual(Buffer.from(expected));
    }
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseStaticImage?.());
    await session.page.evaluate(async () => {
      await (window as any).__staticImageJob; (window as any).__disposeReplacementImage?.(); (window as any).__disposeStaticImage();
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
