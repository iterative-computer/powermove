import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

const frames = 4, width = 128, height = 96;
const ffmpeg = ffmpegBinary;
const formats = [
  { format: 'mp4', alpha: false }, { format: 'prores', alpha: true },
  { format: 'png', alpha: false }, { format: 'png', alpha: true }, { format: 'webm', alpha: false },
];

async function maskProject(session: LaunchedApp, alpha = false, paths = false, grouped = false) {
  await session.page.evaluate(async ({ alpha, paths, grouped }) => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.quality = alpha ? .5 : 1; PM.AutomaticPreview?.cancel();
    const project = PM.mkProject({ name: 'Static masks', w: 128, h: 96, fps: 4, dur: 1, bg: '#21344a' });
    project.colorDepth = 16;
    if (alpha) project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const layer = PM.mkLayer('shape', { d: { w: 113, h: 85, radius: 13, color: '#d7743b' },
      p: { 'position.x': 64, 'position.y': 48, rotation: 7, opacity: 83 } }, project);
    const pathModel = (id: string, w: number, h: number) => ({ id, name: 'Cubic cutout', parent: null,
      p: Object.fromEntries(Object.entries({ x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, closed: true,
        fill: '#ffffff', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2, strokeOpacity: 100,
        trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0 })
        .map(([key, value]) => [key, PM.P(value)])),
      vertices: [[-w / 2, -h / 2], [w / 2, -h / 2], [w / 2, h / 2], [-w / 2, h / 2]]
        .map(([x, y], index) => ({ id: `${id}-vertex-${index}`, p: Object.fromEntries(Object.entries({ x, y,
          inX: index === 0 ? -w * .1 : 0, inY: index === 2 ? h * .15 : 0,
          outX: index === 1 ? w * .12 : 0, outY: index === 3 ? -h * .12 : 0 })
          .map(([key, value]) => [key, PM.P(value)])) })) });
    const add = PM.mkMask('rect', project); add.p.x.v = -5; add.p.w.v = 81; add.p.h.v = 67;
    add.p.rotation.v = 11; add.p.feather.v = 3;
    const subtract = PM.mkMask('rect', project); subtract.mode = 'subtract'; subtract.p.x.v = 13;
    subtract.p.y.v = 5; subtract.p.w.v = 23; subtract.p.h.v = 29; subtract.p.rotation.v = -29; subtract.p.feather.v = 2;
    if (paths) { add.path = pathModel('outer', 81, 67); subtract.path = pathModel('inner', 23, 29); }
    const ellipse = PM.mkMask('ellipse', project); ellipse.p.x.v = -23; ellipse.p.y.v = 11;
    ellipse.p.w.v = 27; ellipse.p.h.v = 35; ellipse.p.rotation.v = 23; ellipse.p.feather.v = 2;
    layer.masks = [add, subtract, ellipse]; let owner = layer;
    if (grouped) { owner = PM.mkLayer('group', { p: { rotation: 13, opacity: 79 } }, project);
      layer.group = owner.id; owner.masks = layer.masks; layer.masks = []; }
    const effectIds = grouped ? ['glow'] : paths ? ['blur', 'shadow'] : [];
    owner.fx = effectIds.map(id => { const fx = PM.mkEffect(id); if (!fx) throw new Error('Missing builtin effect');
      if (id === 'blur') fx.p.amount.v = 5;
      if (id === 'shadow') { fx.p.softness.v = 7; fx.p.distance.v = 5; }
      if (id === 'glow') { fx.p.radius.v = 5; fx.p.intensity.v = 123; } return fx; });
    project.layers = grouped ? [owner, layer] : [layer]; PM.proj = project; PM.touch(); await PM.prepareFrame(0);
    const programs = [...(paths ? ['pathMaskApply'] : ['masks', 'maskApply']), ...effectIds.map(id => 'fx:' + id)];
    PM.GL.renderToPixels(0, 128, 96, { transparent: true, mblur: false, mbSamples: 1, shutter: .5 });
    for (const key of programs) if (!PM.GL.progs.get(key)?.pr || PM.GL.errors.has(key)) throw new Error('Mask preflight failed: ' + key);
    (window as any).__staticMaskFixture = { layer, owner, add, subtract, programs, rotation: structuredClone(layer.p.rotation) };
  }, { alpha, paths, grouped });
}

async function nativeWrites(session: LaunchedApp) {
  await session.app.evaluate(({ ipcMain }) => {
    const original = (ipcMain as any)._invokeHandlers.get('render:write');
    ipcMain.removeHandler('render:write'); ipcMain.handle('render:write', async (...args: any[]) => {
      const state = (globalThis as any).__staticMaskWrites, request = args[1];
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
}

function decodedFrames(input: string, png = false) {
  return execFileSync(ffmpeg, ['-v', 'error', ...(png ? ['-framerate', '4'] : []), '-i', input,
    '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'], { encoding: 'utf8' });
}

for (const { format, alpha } of formats) test(`retains freshly compiled and warm static masks with exact ${alpha ? 'transparent' : 'opaque'} ${format} frames`, async ({ session }) => {
  await session.openEditor(); await maskProject(session, alpha, alpha || format === 'webm', format === 'webm'); await nativeWrites(session);
  let rawReference: string | undefined, decodedReference: string | undefined, pngReference: string[] | undefined;
  let frameReference: Array<{ timestamp: number; duration: number | null; hash: string }> | undefined;
  for (const warm of [false, true]) for (const retained of [false, true]) {
    const folder = path.join(session.userData, `${warm}-${retained}`); await mkdir(folder);
    const output = path.join(folder, `output.${format === 'prores' ? 'mov' : format}`);
    await session.app.evaluate(({ dialog }, { folder, output }) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
      (globalThis as any).__staticMaskWrites = { active: 0, maxActive: 0, writes: 0, repeats: [], last: null,
        hash: process.getBuiltinModule('node:crypto').createHash('sha256') };
    }, { folder, output });
    const result = await session.page.evaluate(async ({ retained, format, alpha, warm }) => {
      const PM = (window as any).PM, fixture = (window as any).__staticMaskFixture;
      fixture.layer.p.rotation = structuredClone(fixture.rotation);
      // The baseline must keep the real PBO path enabled. Two identical keys
      // force per-frame capture while preserving identical rendered geometry.
      if (!retained) PM.animate(fixture.layer, 'rotation', [{ t: 0, v: 7 }, { t: 1, v: 7 }]);
      PM.touch(); await PM.prepareFrame(0);
      if (!warm) for (const key of fixture.programs) PM.GL.dropProgram(key);
      PM.GL.renderToPixels(0, 128, 96, { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
      for (const key of fixture.programs) if (!PM.GL.progs.get(key)?.pr || PM.GL.errors.has(key))
        throw new Error('Static mask program preflight failed: ' + key);
      const pixels = PM.GL.renderToPixels, render = PM.GL.render, encode = HTMLCanvasElement.prototype.toBlob;
      const videoEncode = VideoEncoder.prototype.encode;
      const factories = ['createOpaqueReadback', 'createAlphaReadback', 'createHalfReadback', 'createFloatReadback'].map(name => [name, PM.GL[name]] as const);
      const state = { captures: 0, presents: 0, encodes: 0, sessions: 0, disposed: 0, activeSessions: 0, maxSessions: 0, pending: 0, maxPending: 0, alphaAsyncFactories: 0, copiedClosed: 0, captureQualities: [] as number[] };
      const copies: Promise<any>[] = [], targets = new Set<Uint8Array>();
      PM.GL.renderToPixels = (...args: any[]) => { state.captures++; state.captureQualities.push(PM.quality); return pixels(...args); };
      PM.GL.render = (...args: any[]) => { state.presents++; return render(...args); };
      for (const [name, factory] of factories) if (factory) PM.GL[name] = (...args: any[]) => {
        if (name === 'createAlphaReadback') state.alphaAsyncFactories++;
        const capture = factory(...args); if (!capture) return capture; state.sessions++; state.maxSessions = Math.max(state.maxSessions, ++state.activeSessions);
        return name === 'createOpaqueReadback' || name === 'createAlphaReadback' ? {
          get byteLength() { return capture.byteLength; },
          begin(...args: any[]) {
            state.captures++; state.captureQualities.push(PM.quality);
            const ticket = capture.begin(...args); if (!ticket) return ticket;
            state.maxPending = Math.max(state.maxPending, ++state.pending); let disposed = false;
            return { readInto(target: Uint8Array, cancelled: () => boolean) { targets.add(target); return ticket.readInto(target, cancelled); },
              dispose() { if (!disposed) { disposed = true; state.pending--; } ticket.dispose(); } };
          },
          dispose() { state.disposed++; state.activeSessions--; capture.dispose(); },
        } : {
          capture(...args: any[]) {
            const before = state.captures, result = capture.capture(...args);
            if (before === state.captures) { state.captures++; state.captureQualities.push(PM.quality); } return result;
          }, dispose() { state.disposed++; state.activeSessions--; capture.dispose(); },
        };
      };
      HTMLCanvasElement.prototype.toBlob = function (...args: any[]) { state.encodes++; return (encode as any).apply(this, args); };
      VideoEncoder.prototype.encode = function (frame: VideoFrame, options?: VideoEncoderEncodeOptions) {
        const copy = frame.clone(), bytes = new Uint8Array(copy.allocationSize({ format: 'RGBA' }));
        const timestamp = copy.timestamp, duration = copy.duration;
        copies.push(copy.copyTo(bytes, { format: 'RGBA' }).then(async () => ({ timestamp, duration,
          hash: Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))).map(byte => byte.toString(16).padStart(2, '0')).join(''),
        })).finally(() => { copy.close(); state.copiedClosed++; }).catch(error => ({ error: String(error) })));
        return videoEncode.call(this, frame, options);
      };
      const before = PM.serialize(), time = PM.time, quality = PM.quality;
      try {
        const result = await PM.Export.run({ format, alpha, scale: 1, fps: 4, range: 'all', quality: 'draft', audio: false, mblur: false });
        const videoFrames = await Promise.all(copies);
        return { result, ...state, targets: targets.size, videoFrames, prepared: PM.preparedVideoFrames,
          preserved: before === PM.serialize() && time === PM.time && quality === PM.quality };
      } finally {
        await Promise.allSettled(copies);
        PM.GL.renderToPixels = pixels; PM.GL.render = render; HTMLCanvasElement.prototype.toBlob = encode; VideoEncoder.prototype.encode = videoEncode;
        for (const [name, factory] of factories) PM.GL[name] = factory;
      }
    }, { retained, format, alpha, warm });
    expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true); expect(result.prepared).toBeNull();
    expect(result.disposed).toBe(result.sessions); expect(result.activeSessions).toBe(0); expect(result.pending).toBe(0);
    expect(result.maxSessions).toBeLessThanOrEqual(1); expect(result.maxPending).toBeLessThanOrEqual(1); expect(result.targets).toBeLessThanOrEqual(1);
    if (format === 'webm') {
      expect(result.presents).toBe(retained ? 1 : frames); expect(result.copiedClosed).toBe(frames);
      expect(result.videoFrames.map(frame => frame.timestamp)).toEqual([0, 250000, 500000, 750000]);
      expect(result.videoFrames.every(frame => frame.duration === 250000)).toBe(true);
      if (!frameReference) frameReference = result.videoFrames; else expect(result.videoFrames).toEqual(frameReference);
    } else expect(result.captures).toBe(retained ? 1 : frames);
    if (format === 'png') expect(result.encodes).toBe(retained ? 1 : frames);
    if (alpha) {
      expect(result.captureQualities).toEqual(Array(retained ? 1 : frames).fill(.5));
      if (retained) expect(result.alphaAsyncFactories).toBe(0);
    }
    if (format === 'mp4' || format === 'prores') {
      const main = await session.app.evaluate(() => {
        const state = (globalThis as any).__staticMaskWrites;
        return { active: state.active, maxActive: state.maxActive, writes: state.writes, repeats: state.repeats, hash: state.hash.digest('hex') };
      });
      expect(main).toMatchObject({ active: 0, maxActive: 1, writes: retained ? 1 : frames, repeats: retained ? [1, 1, 1] : [] });
      if (rawReference === undefined) rawReference = main.hash; else expect(main.hash).toBe(rawReference);
    }
    const input = format === 'png' ? path.join(folder, 'Static masks_%05d.png') : output;
    const decoded = decodedFrames(input, format === 'png');
    expect(decoded.split('\n').filter(line => line && !line.startsWith('#'))).toHaveLength(frames);
    if (decodedReference === undefined) decodedReference = decoded; else expect(decoded).toBe(decodedReference);
    if (format === 'png') {
      const hashes = await Promise.all(Array.from({ length: frames }, async (_, i) => createHash('sha256')
        .update(await readFile(path.join(folder, `Static masks_${String(i).padStart(5, '0')}.png`))).digest('hex')));
      if (pngReference === undefined) pngReference = hashes; else expect(hashes).toEqual(pngReference);
    }
    const first = execFileSync(ffmpeg, ['-v', 'error', ...(format === 'png' ? ['-framerate', '4'] : []), '-i', input,
      '-frames:v', '1', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
    const colors = new Set<number>(); for (let at = 0; at < first.length; at += 4) colors.add(first.readUInt32LE(at));
    expect(colors.size).toBeGreaterThan(32);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const change of ['vertex', 'program'] as const) test(`static masks recapture the same frame after a ${change} edit during a held PNG save`, async ({ session }) => {
  await session.openEditor(); await maskProject(session, false, change === 'vertex');
  const folder = path.join(session.userData, 'changed'); await mkdir(folder);
  const expected = await session.page.evaluate(async change => {
    const PM = (window as any).PM, fixture = (window as any).__staticMaskFixture, gl = PM.GL.gl;
    const property = fixture.add.path?.vertices[0].p.x, originalValue = property?.v;
    const originalProgram = PM.GL.progs.get('maskApply');
    let customProgram: WebGLProgram | null = null, replacement: any;
    if (change === 'program') {
      const old = 'o = c * texture(u_cov, v_st).r;', source = PM.FRAG_MASK_APPLY;
      if (!source.includes(old)) throw new Error('Builtin mask output changed');
      const fragment = source.replace(old, 'o = vec4(c.rgb*vec3(.1,.8,.3),c.a) * texture(u_cov, v_st).r;');
      const compile = (type: number, source: string) => {
        const shader = gl.createShader(type); if (!shader) throw new Error('No shader');
        gl.shaderSource(shader, source); gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
        return shader;
      };
      const vertex = compile(gl.VERTEX_SHADER, PM.VERT), pixel = compile(gl.FRAGMENT_SHADER,
        fragment.replace(/^(#version[^\n]*\n)/, '$1precision highp sampler2D;\n'));
      customProgram = gl.createProgram(); if (!customProgram) throw new Error('No program');
      gl.attachShader(customProgram, vertex); gl.attachShader(customProgram, pixel); gl.linkProgram(customProgram);
      gl.deleteShader(vertex); gl.deleteShader(pixel);
      if (!gl.getProgramParameter(customProgram, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(customProgram));
      replacement = { ...originalProgram, pr: customProgram, u: new Map(), a: gl.getAttribLocation(customProgram, 'a_pos') };
    }
    const replace = () => {
      if (change === 'vertex') { property.v = originalValue + 31; PM.touch(); }
      else PM.GL.progs.set('maskApply', replacement);
    };
    const restore = () => {
      if (change === 'vertex') { property.v = originalValue; PM.touch(); }
      else PM.GL.progs.set('maskApply', originalProgram);
    };
    const reference: number[] = [], pictures: Uint8Array[] = [];
    try {
      for (let frame = 0; frame < 4; frame++) {
        if (frame === 1) replace(); await PM.prepareFrame(frame / 4);
        const canvas = PM.renderFrameTo(frame / 4, 128, 96, { mblur: false });
        const pixels = new Uint8Array(canvas.getContext('2d').getImageData(0, 0, 128, 96).data);
        canvas.width = 0; canvas.height = 0; pictures.push(pixels); reference.push(...pixels);
      }
      if (pictures[0]!.every((byte, index) => byte === pictures[1]![index])) throw new Error('Edit must visibly change the mask picture');
    } finally { restore(); await PM.prepareFrame(0); }
    (window as any).__changeStaticMask = replace;
    (window as any).__restoreStaticMask = () => { restore(); if (customProgram) gl.deleteProgram(customProgram); };
    return reference;
  }, change);
  try {
    await session.app.evaluate(({ ipcMain, dialog }, folder) => {
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
      const original = (ipcMain as any)._invokeHandlers.get('file:save'), state = { held: false, writes: 0 };
      (globalThis as any).__heldStaticMask = state;
      ipcMain.removeHandler('file:save'); ipcMain.handle('file:save', async (...args: any[]) => {
        state.writes++; const result = await original(...args);
        if (state.writes === 1) {
          state.held = true; await new Promise<void>(resolve => { (globalThis as any).__releaseStaticMask = resolve; }); state.held = false;
        }
        return result;
      });
    }, folder);
    await session.page.evaluate(() => {
      const PM = (window as any).PM, render = PM.renderFrameTo, captures: number[] = [];
      (window as any).__staticMaskCaptures = captures;
      PM.renderFrameTo = (...args: any[]) => { captures.push(args[0]); return render(...args); };
      (window as any).__restoreStaticMaskCapture = () => { PM.renderFrameTo = render; };
      const project = PM.proj, time = PM.time, quality = PM.quality;
      (window as any).__staticMaskHeldResult = null;
      (window as any).__staticMaskHeldJob = PM.Export.run({ format: 'png', alpha: false, scale: 1, fps: 4,
        range: 'all', quality: 'draft', audio: false, mblur: false }).then((result: any) => {
          const value = { result, source: PM.serialize(), preserved: project === PM.proj && time === PM.time && quality === PM.quality,
            prepared: PM.preparedVideoFrames }; (window as any).__staticMaskHeldResult = value; return value;
        });
    });
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__heldStaticMask.held)).toBe(true);
    const changedSource = await session.page.evaluate(() => { (window as any).__changeStaticMask(); return (window as any).PM.serialize(); });
    expect(await session.page.evaluate(() => (window as any).__staticMaskHeldResult)).toBeNull();
    await session.app.evaluate(() => (globalThis as any).__releaseStaticMask());
    expect(await session.page.evaluate(() => (window as any).__staticMaskHeldJob))
      .toEqual({ result: { cancelled: false }, source: changedSource, preserved: true, prepared: null });
    expect(await session.page.evaluate(() => (window as any).__staticMaskCaptures)).toEqual([0, .25]);
    const decoded = execFileSync(ffmpeg, ['-v', 'error', '-framerate', '4', '-i', path.join(folder, 'Static masks_%05d.png'),
      '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
    expect(decoded).toEqual(Buffer.from(expected));
    expect(decodedFrames(path.join(folder, 'Static masks_%05d.png'), true).split('\n')
      .filter(line => line && !line.startsWith('#'))).toHaveLength(frames);
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseStaticMask?.());
    await session.page.evaluate(async () => {
      await (window as any).__staticMaskHeldJob; (window as any).__restoreStaticMaskCapture?.(); (window as any).__restoreStaticMask?.();
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
