import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, repoRoot, type LaunchedApp } from './helpers/app';

const frames = 4, width = 128, height = 96;
const ffmpeg = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
const formats = [
  { format: 'mp4', alpha: false }, { format: 'prores', alpha: true },
  { format: 'png', alpha: false }, { format: 'png', alpha: true }, { format: 'webm', alpha: false },
];

async function effectProject(session: LaunchedApp, alpha = false, effect = 'blur') {
  await session.page.evaluate(async ({ alpha, effect }) => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.quality = alpha ? .5 : 1; PM.AutomaticPreview?.cancel();
    const project = PM.mkProject({ name: 'Static effects', w: 128, h: 96, fps: 4, dur: 1, bg: '#21344a' });
    project.colorDepth = 16;
    if (alpha) project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const layer = PM.mkLayer('shape', { from: 0, dur: 1, d: { w: 83, h: 61, radius: 17, color: '#ac4fea' },
      p: { 'position.x': 56, 'position.y': 42, rotation: 7, opacity: 71 } }, project);
    const bottom = PM.mkLayer('shape', { from: 0, dur: 1, d: { shape: 'ellipse', w: 59, h: 47, color: '#e8a43b' },
      p: { 'position.x': 84, 'position.y': 63, opacity: 83 } }, project);
    const fx = PM.mkEffect(effect); if (!fx) throw new Error('Effect unavailable');
    if (effect === 'blur') fx.p.amount.v = 13;
    if (effect === 'grain') { fx.p.amount.v = 23; fx.p.size.v = 1.7; }
    if (effect === 'displace') { fx.p.amount.v = 9; fx.p.scale.v = 1.7; fx.p.speed.v = .73; }
    layer.fx = [fx]; project.layers = [layer, bottom]; PM.proj = project; PM.touch();
    await PM.prepareFrame(0);
    // Read the actual compiled program, not a placeholder that a cold viewer
    // may temporarily show while its asynchronous shader compilation finishes.
    PM.GL.renderToPixels(0, 128, 96, { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
    const linked = PM.GL.progs.get('fx:' + effect), gl = PM.GL.gl;
    if (!linked?.pr || !gl.getProgramParameter(linked.pr, gl.LINK_STATUS) || PM.GL.errors.has('fx:' + effect))
      throw new Error('Expected an actual compiled effect program');
    (window as any).__staticEffectFixture = { layer, fx, effect, rotation: structuredClone(layer.p.rotation) };
  }, { alpha, effect });
}

async function nativeWrites(session: LaunchedApp) {
  await session.app.evaluate(({ ipcMain }) => {
    const original = (ipcMain as any)._invokeHandlers.get('render:write');
    ipcMain.removeHandler('render:write'); ipcMain.handle('render:write', async (...args: any[]) => {
      const state = (globalThis as any).__staticEffectWrites, request = args[1];
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

for (const { format, alpha } of formats) test(`retains freshly compiled and warm static effects with exact ${alpha ? 'transparent' : 'opaque'} ${format} frames`, async ({ session }) => {
  await session.openEditor(); await effectProject(session, alpha); await nativeWrites(session);
  let rawReference: string | undefined, decodedReference: string | undefined, pngReference: string[] | undefined;
  let frameReference: Array<{ timestamp: number; duration: number | null; hash: string }> | undefined;
  for (const warm of [false, true]) for (const retained of [false, true]) {
    const folder = path.join(session.userData, `${warm}-${retained}`); await mkdir(folder);
    const output = path.join(folder, `output.${format === 'prores' ? 'mov' : format}`);
    await session.app.evaluate(({ dialog }, { folder, output }) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
      (globalThis as any).__staticEffectWrites = { active: 0, maxActive: 0, writes: 0, repeats: [], last: null,
        hash: process.getBuiltinModule('node:crypto').createHash('sha256') };
    }, { folder, output });
    const result = await session.page.evaluate(async ({ retained, format, alpha, warm }) => {
      const PM = (window as any).PM, fixture = (window as any).__staticEffectFixture;
      fixture.layer.p.rotation = structuredClone(fixture.rotation);
      // The baseline must keep the real PBO path enabled. Two identical keys
      // force per-frame capture while preserving identical rendered geometry.
      if (!retained) PM.animate(fixture.layer, 'rotation', [{ t: 0, v: 7 }, { t: 1, v: 7 }]);
      PM.touch(); await PM.prepareFrame(0);
      if (!warm) PM.GL.dropProgram('fx:' + fixture.effect);
      PM.GL.renderToPixels(0, 128, 96, { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
      const linked = PM.GL.progs.get('fx:' + fixture.effect);
      if (!linked?.pr || PM.GL.gl.getUniformLocation(linked.pr, 'u_time') !== null
          || PM.GL.gl.getUniformLocation(linked.pr, 'u_prog') !== null) throw new Error('Static program preflight failed');
      const pixels = PM.GL.renderToPixels, render = PM.GL.render, encode = HTMLCanvasElement.prototype.toBlob;
      const videoEncode = VideoEncoder.prototype.encode;
      const factories = ['createOpaqueReadback', 'createHalfReadback', 'createFloatReadback'].map(name => [name, PM.GL[name]] as const);
      const state = { captures: 0, presents: 0, encodes: 0, sessions: 0, disposed: 0, copiedClosed: 0, captureQualities: [] as number[] };
      const copies: Promise<any>[] = [];
      PM.GL.renderToPixels = (...args: any[]) => { state.captures++; state.captureQualities.push(PM.quality); return pixels(...args); };
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
        return { result, ...state, videoFrames, prepared: PM.preparedVideoFrames,
          preserved: before === PM.serialize() && time === PM.time && quality === PM.quality };
      } finally {
        await Promise.allSettled(copies);
        PM.GL.renderToPixels = pixels; PM.GL.render = render; HTMLCanvasElement.prototype.toBlob = encode; VideoEncoder.prototype.encode = videoEncode;
        for (const [name, factory] of factories) PM.GL[name] = factory;
      }
    }, { retained, format, alpha, warm });
    expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true); expect(result.prepared).toBeNull();
    expect(result.disposed).toBe(result.sessions);
    if (format === 'webm') {
      expect(result.presents).toBe(retained ? 1 : frames); expect(result.copiedClosed).toBe(frames);
      expect(result.videoFrames.map(frame => frame.timestamp)).toEqual([0, 250000, 500000, 750000]);
      expect(result.videoFrames.every(frame => frame.duration === 250000)).toBe(true);
      if (!frameReference) frameReference = result.videoFrames; else expect(result.videoFrames).toEqual(frameReference);
    } else expect(result.captures).toBe(retained ? 1 : frames);
    if (format === 'png') expect(result.encodes).toBe(retained ? 1 : frames);
    if (alpha) expect(result.captureQualities).toEqual(Array(retained ? 1 : frames).fill(.5));
    if (format === 'mp4' || format === 'prores') {
      const main = await session.app.evaluate(() => {
        const state = (globalThis as any).__staticEffectWrites;
        return { active: state.active, maxActive: state.maxActive, writes: state.writes, repeats: state.repeats, hash: state.hash.digest('hex') };
      });
      expect(main).toMatchObject({ active: 0, maxActive: 1, writes: retained ? 1 : frames, repeats: retained ? [1, 1, 1] : [] });
      if (rawReference === undefined) rawReference = main.hash; else expect(main.hash).toBe(rawReference);
    }
    const input = format === 'png' ? path.join(folder, 'Static effects_%05d.png') : output;
    const decoded = decodedFrames(input, format === 'png');
    expect(decoded.split('\n').filter(line => line && !line.startsWith('#'))).toHaveLength(frames);
    if (decodedReference === undefined) decodedReference = decoded; else expect(decoded).toBe(decodedReference);
    if (format === 'png') {
      const hashes = await Promise.all(Array.from({ length: frames }, async (_, i) => createHash('sha256')
        .update(await readFile(path.join(folder, `Static effects_${String(i).padStart(5, '0')}.png`))).digest('hex')));
      if (pngReference === undefined) pngReference = hashes; else expect(hashes).toEqual(pngReference);
    }
    const first = execFileSync(ffmpeg, ['-v', 'error', ...(format === 'png' ? ['-framerate', '4'] : []), '-i', input,
      '-frames:v', '1', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
    const colors = new Set<number>(); for (let at = 0; at < first.length; at += 4) colors.add(first.readUInt32LE(at));
    expect(colors.size).toBeGreaterThan(32);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const change of ['default', 'registration', 'program', 'revision'] as const) test(`static effects recapture the same frame after a ${change} change during a held ${change === 'default' ? 'encoder ACK' : 'PNG save'}`, async ({ session }) => {
  await session.openEditor(); await effectProject(session);
  const format = change === 'default' ? 'mp4' : 'png', folder = path.join(session.userData, 'changed'); await mkdir(folder);
  const expected = await session.page.evaluate(async ({ change, format }) => {
    const PM = (window as any).PM, fixture = (window as any).__staticEffectFixture, gl = PM.GL.gl;
    const definition = PM.Kernel.effects.get('blur'), parameter = definition.params[0], oldDefault = parameter.def;
    if (change === 'revision') fixture.fx.version = 1;
    if (change === 'default') { delete fixture.fx.p.amount; PM.touch(); }
    const replacement = structuredClone(definition), output = 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));';
    if (!replacement.frag.includes(output)) throw new Error('Builtin blur output changed');
    replacement.frag = replacement.frag.replace(output, 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));o.rgb*=vec3(.1,.8,.3);');
    const originalProgram = PM.GL.progs.get('fx:blur');
    let customProgram: WebGLProgram | null = null, record: any, registration: any;
    if (change === 'program') {
      const compile = (type: number, source: string) => {
        const shader = gl.createShader(type); if (!shader) throw new Error('No shader');
        gl.shaderSource(shader, source); gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
        return shader;
      };
      const vertex = compile(gl.VERTEX_SHADER, PM.VERT), fragment = compile(gl.FRAGMENT_SHADER,
        replacement.frag.replace(/^(#version[^\n]*\n)/, '$1precision highp sampler2D;\n'));
      customProgram = gl.createProgram(); if (!customProgram) throw new Error('No program');
      gl.attachShader(customProgram, vertex); gl.attachShader(customProgram, fragment); gl.linkProgram(customProgram);
      gl.deleteShader(vertex); gl.deleteShader(fragment);
      if (!gl.getProgramParameter(customProgram, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(customProgram));
      record = { ...originalProgram, pr: customProgram, u: new Map(), a: gl.getAttribLocation(customProgram, 'a_pos') };
    }
    const replace = () => {
      if (change === 'revision') fixture.fx.version = 2;
      else if (change === 'default') parameter.def = 89;
      else if (change === 'registration') registration = PM.Kernel.registerEffect('effects-basic', replacement);
      else PM.GL.progs.set('fx:blur', record);
    };
    const restore = () => {
      if (change === 'revision') fixture.fx.version = 1;
      else if (change === 'default') parameter.def = oldDefault;
      else if (change === 'registration') { registration?.dispose(); registration = undefined; }
      else PM.GL.progs.set('fx:blur', originalProgram);
    };
    const reference: number[] = [], pictures: Uint8Array[] = [];
    try {
      for (let frame = 0; frame < 4; frame++) {
        if (frame === 1) replace();
        await PM.prepareFrame(frame / 4);
        let pixels: Uint8Array;
        if (format === 'mp4') pixels = PM.GL.renderToPixels(frame / 4, 128, 96,
          { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
        else {
          const canvas = PM.renderFrameTo(frame / 4, 128, 96, { mblur: false });
          pixels = new Uint8Array(canvas.getContext('2d').getImageData(0, 0, 128, 96).data);
          canvas.width = 0; canvas.height = 0;
        }
        pictures.push(pixels); reference.push(...pixels);
      }
      if (pictures[0]!.every((byte, index) => byte === pictures[1]![index])) throw new Error('Replacement must visibly change the static picture');
    } finally {
      restore(); await PM.prepareFrame(0);
      // A registration restore drops compiled programs. Establish a real old
      // program again so the export begins with an eligible static picture.
      PM.GL.renderToPixels(0, 128, 96, { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
    }
    (window as any).__changeStaticEffect = replace;
    (window as any).__restoreStaticEffect = () => { restore(); if (customProgram) gl.deleteProgram(customProgram); };
    return reference;
  }, { change, format });
  try {
    await session.app.evaluate(({ ipcMain, dialog }, { folder, format }) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: `${folder}/changed.mp4` });
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
      const channel = format === 'mp4' ? 'render:write' : 'file:save', original = (ipcMain as any)._invokeHandlers.get(channel);
      const state = { held: false, writes: 0, chunks: [] as number[][] };
      (globalThis as any).__heldStaticEffect = state;
      ipcMain.removeHandler(channel); ipcMain.handle(channel, async (...args: any[]) => {
        state.writes++; const request = args[1];
        if (format === 'mp4') {
          if (request.repeat !== undefined) for (let i = 0; i < request.repeat; i++) state.chunks.push(state.chunks.at(-1)!);
          else state.chunks.push(Array.from(request.data));
        }
        const result = await original(...args);
        if (state.writes === 1) {
          state.held = true; await new Promise<void>(resolve => { (globalThis as any).__releaseStaticEffect = resolve; }); state.held = false;
        }
        return result;
      });
    }, { folder, format });
    await session.page.evaluate(format => {
      const PM = (window as any).PM, pixels = PM.GL.renderToPixels, render = PM.renderFrameTo;
      const captures: number[] = []; (window as any).__staticEffectCaptures = captures;
      PM.GL.renderToPixels = (...args: any[]) => { if (format === 'mp4') captures.push(args[0]); return pixels(...args); };
      PM.renderFrameTo = (...args: any[]) => { if (format === 'png') captures.push(args[0]); return render(...args); };
      (window as any).__restoreStaticCapture = () => { PM.GL.renderToPixels = pixels; PM.renderFrameTo = render; };
      const before = PM.serialize(), time = PM.time, quality = PM.quality;
      (window as any).__staticEffectHeldResult = null;
      (window as any).__staticEffectHeldJob = PM.Export.run({ format, alpha: false, scale: 1, fps: 4,
        range: 'all', quality: 'draft', audio: false, mblur: false }).then((result: any) => {
          const value = { result, preserved: before === PM.serialize() && time === PM.time && quality === PM.quality,
            prepared: PM.preparedVideoFrames }; (window as any).__staticEffectHeldResult = value; return value;
        });
    }, format);
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__heldStaticEffect.held)).toBe(true);
    await session.page.evaluate(() => (window as any).__changeStaticEffect());
    expect(await session.page.evaluate(() => (window as any).__staticEffectHeldResult)).toBeNull();
    await session.app.evaluate(() => (globalThis as any).__releaseStaticEffect());
    expect(await session.page.evaluate(() => (window as any).__staticEffectHeldJob))
      .toEqual({ result: { cancelled: false }, preserved: change !== 'revision', prepared: null });
    // Registration drops the linked program. The first changed picture
    // compiles it, and only the following picture can start a retained run.
    expect(await session.page.evaluate(() => (window as any).__staticEffectCaptures))
      .toEqual(change === 'registration' ? [0, .25, .5] : [0, .25]);
    if (format === 'mp4') expect(await session.app.evaluate(() => (globalThis as any).__heldStaticEffect.chunks.flat())).toEqual(expected);
    else {
      const decoded = execFileSync(ffmpeg, ['-v', 'error', '-framerate', '4', '-i', path.join(folder, 'Static effects_%05d.png'),
        '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
      expect(decoded).toEqual(Buffer.from(expected));
    }
    const decoded = decodedFrames(format === 'mp4' ? path.join(folder, 'changed.mp4') : path.join(folder, 'Static effects_%05d.png'), format === 'png');
    expect(decoded.split('\n').filter(line => line && !line.startsWith('#'))).toHaveLength(frames);
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseStaticEffect?.());
    await session.page.evaluate(async () => {
      await (window as any).__staticEffectHeldJob; (window as any).__restoreStaticCapture?.(); (window as any).__restoreStaticEffect?.();
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const effect of ['grain', 'displace', 'same-owner-time'] as const) test(`does not retain static geometry with ${effect} temporal shader input`, async ({ session }) => {
  await session.openEditor(); await effectProject(session, false, effect === 'same-owner-time' ? 'blur' : effect);
  const output = path.join(session.userData, `${effect}.mp4`);
  await session.app.evaluate(({ dialog }, output) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: output }); }, output);
  const result = await session.page.evaluate(async effect => {
    const PM = (window as any).PM, fixture = (window as any).__staticEffectFixture;
    let registration: any;
    if (effect === 'same-owner-time') {
      const definition = structuredClone(PM.Kernel.effects.get('blur')), old = 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));';
      if (!definition.frag.includes(old)) throw new Error('Builtin blur output changed');
      definition.frag = definition.frag.replace('uniform float u_sampleLod;', 'uniform float u_sampleLod; uniform float u_time;')
        .replace(old, 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));o.rgb*=.3+.7*cos(u_time*3.);');
      registration = PM.Kernel.registerEffect('effects-basic', definition);
    }
    await PM.prepareFrame(0);
    PM.GL.renderToPixels(0, 128, 96, { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
    const linked = PM.GL.progs.get('fx:' + fixture.effect);
    if (!linked?.pr || PM.GL.gl.getUniformLocation(linked.pr, 'u_time') === null) throw new Error('Expected live temporal uniform');
    const pixels = PM.GL.renderToPixels, factory = PM.GL.createOpaqueReadback;
    const state = { captures: [] as number[], sessions: 0, disposed: 0 };
    PM.GL.renderToPixels = (...args: any[]) => { state.captures.push(args[0]); return pixels(...args); };
    PM.GL.createOpaqueReadback = (...args: any[]) => {
      const capture = factory(...args); if (!capture) return capture; state.sessions++;
      return { begin(...args: any[]) { state.captures.push(args[0]); return capture.begin(...args); },
        dispose() { state.disposed++; capture.dispose(); } };
    };
    const before = PM.serialize(), time = PM.time, quality = PM.quality;
    try {
      const result = await PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4, range: 'all', quality: 'draft', audio: false, mblur: false });
      return { result, ...state, preserved: before === PM.serialize() && time === PM.time && quality === PM.quality };
    } finally { PM.GL.renderToPixels = pixels; PM.GL.createOpaqueReadback = factory; registration?.dispose(); }
  }, effect);
  expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true);
  expect(result.captures).toEqual([0, .25, .5, .75]); expect(result.disposed).toBe(result.sessions);
  const decoded = decodedFrames(output), lines = decoded.split('\n').filter(line => line && !line.startsWith('#'));
  expect(lines).toHaveLength(frames); expect(new Set(lines.map(line => line.split(',').at(-1))).size).toBeGreaterThan(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
