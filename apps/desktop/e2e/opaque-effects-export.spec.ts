import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot, type LaunchedApp } from './helpers/app';

const cases: Array<{ name: string; effects: string[]; depth: number; masks: boolean; group: boolean; paths?: boolean }> = [
  { name: 'blur', effects: ['blur'], depth: 16, masks: false, group: false },
  { name: 'glow', effects: ['glow'], depth: 32, masks: false, group: false },
  { name: 'shadow', effects: ['shadow'], depth: 8, masks: false, group: false },
  ...['motionblurDir', 'sharpen', 'color', 'levels', 'duotone', 'grain', 'vignette',
    'chroma', 'pixelate', 'posterize', 'displace', 'invert', 'gradient', 'tint', 'channelMixer', 'threshold', 'shiftChannels', 'exposure', 'levelsIndividual', 'unsharpMask', 'findEdges', 'tritone', 'colorBalanceHLS', 'brightnessContrast', 'blackWhite'].map((id, index) => ({
      name: id, effects: [id], depth: [8, 16, 32][index % 3]!, masks: false, group: false,
    })),
  { name: 'blur and shadow', effects: ['blur', 'shadow'], depth: 16, masks: false, group: false },
  { name: 'animated rectangle and ellipse masks', effects: [], depth: 16, masks: true, group: false },
  { name: 'group glow and masks', effects: ['glow'], depth: 32, masks: true, group: true },
  { name: 'animated cubic path and ellipse masks', effects: [], depth: 8, masks: true, group: false, paths: true },
  { name: 'path masks with blur and shadow', effects: ['blur', 'shadow'], depth: 16, masks: true, group: false, paths: true },
  { name: 'group glow and path masks', effects: ['glow'], depth: 32, masks: true, group: true, paths: true },
];

async function effectProject(session: LaunchedApp, fixture = cases[0]!) {
  return session.page.evaluate(async fixture => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.AutomaticPreview?.cancel();
    const project = PM.mkProject({ name: 'Opaque effects', w: 128, h: 96, fps: 4, dur: 1, bg: '#21344a' });
    project.colorDepth = fixture.depth;
    const shape = PM.mkLayer('shape', { d: { w: 83, h: 63, radius: 17, color: '#ac4fea' },
      p: { 'position.x': 58, 'position.y': 43, rotation: 11, opacity: 71 } }, project);
    const bottom = PM.mkLayer('shape', { d: { shape: 'ellipse', w: 57, h: 43, color: '#e8a43b' },
      p: { 'position.x': 83, 'position.y': 62, opacity: 81 } }, project);
    const owner = fixture.group ? PM.mkLayer('group', { p: { rotation: 7, opacity: 83 } }, project) : shape;
    if (fixture.group) { shape.group = owner.id; bottom.group = owner.id; }
    owner.fx = fixture.effects.map(type => {
      const effect = PM.mkEffect(type,undefined,owner);
      if (!effect || PM.Kernel.effects.topEntry(type)?.ownerId !== 'effects-basic') throw new Error(`Builtin ${type} unavailable`);
      if (type === 'blur') effect.p.amount.v = 13;
      if (type === 'glow') { effect.p.radius.v = 17; effect.p.threshold.v = 30; effect.p.intensity.v = 120; }
      if (type === 'shadow') { effect.p.softness.v = 19; effect.p.distance.v = 11; }
      if (type === 'displace') { effect.p.amount.v = 9; effect.p.scale.v = 1.7; effect.p.speed.v = .73; }
      if (type === 'grain') { effect.p.amount.v = 21; effect.p.size.v = 1.4; }
      if (type === 'chroma') effect.p.amount.v = 3;
      if (type === 'pixelate') { effect.p.horizontalBlocks.v = 11; effect.p.verticalBlocks.v = 7; }
      if (type === 'gradient') {
        effect.p.startColor.v = '#f13b88'; effect.p.endColor.v = '#2bbba8';
        effect.p.rampScatter.v = 3; effect.p.startX.v = 9; effect.p.startY.v = 7; effect.p.endX.v = 70; effect.p.endY.v = 57;
      }
      return effect;
    });
    if (fixture.masks) {
      const ellipse = PM.mkMask('ellipse', project), rectangle = PM.mkMask('rect', project);
      ellipse.p.w.v = 91; ellipse.p.h.v = 71; ellipse.p.feather.v = 9;
      rectangle.mode = 'subtract'; rectangle.p.x.v = 21; rectangle.p.w.v = 23; rectangle.p.h.v = 29; rectangle.p.feather.v = 5;
      owner.masks = [ellipse, rectangle];
      if (fixture.paths) {
        const path = { id: 'export-mask-path', name: 'Cubic mask', parent: null,
          p: Object.fromEntries(Object.entries({ x: 0, y: 0, rotation: 13, scaleX: 100, scaleY: 100,
            closed: true, fill: '#ffffff', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff',
            strokeWidth: 2, strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0,
            copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0 }).map(([key, value]) => [key, PM.P(value)])),
          vertices: [[-44, -33], [44, -33], [44, 33], [-44, 33]].map(([x, y], index) => ({
            id: `export-mask-vertex-${index}`, p: Object.fromEntries(Object.entries({ x, y,
              inX: index === 0 ? -11 : 0, inY: index === 2 ? 9 : 0,
              outX: index === 1 ? 13 : 0, outY: index === 3 ? -7 : 0 }).map(([key, value]) => [key, PM.P(value)])),
          })),
        };
        rectangle.path = path; rectangle.p.x.v = 0; rectangle.p.y.v = 0;
        rectangle.mode = 'add'; ellipse.mode = 'subtract'; ellipse.p.w.v = 29; ellipse.p.h.v = 37;
        owner.masks = [rectangle, ellipse];
      }
    }
    project.layers = fixture.group ? [owner, shape, bottom] : [shape, bottom]; PM.proj = project; PM.touch();
    PM.animate(shape, 'rotation', [{ t: 0, v: -11 }, { t: 1, v: 23 }]);
    for (const effect of owner.fx) {
      if (effect.type === 'gradient') {
        PM.animate(owner, 'effect.angle', [{ t: 0, v: -23 }, { t: 1, v: 129 }], { prop: effect.p.angle });
        PM.animate(owner, 'effect.radial', [{ t: 0, v: false }, { t: .5, v: true }], { prop: effect.p.radial });
        PM.animate(owner, 'effect.shade', [{ t: 0, v: false }, { t: .5, v: true }], { prop: effect.p.shade });
        PM.animate(owner, 'effect.color', [{ t: 0, v: '#f13b88' }, { t: .5, v: '#f3c524' }], { prop: effect.p.startColor });
      } else if (['blur', 'glow', 'shadow'].includes(effect.type)) {
        const key = effect.type === 'blur' ? 'amount' : effect.type === 'glow' ? 'radius' : 'softness';
        PM.animate(owner, 'effect.value', [{ t: 0, v: 9 }, { t: 1, v: 23 }], { prop: effect.p[key] });
      } else {
        const parameter = PM.Kernel.effects.get(effect.type).params.find((param: any) => !param.type || param.type === 'number');
        if (!parameter) throw new Error(`Expected a numeric builtin parameter: ${effect.type}`);
        const start = Number(effect.p[parameter.k].v), delta = Math.max(.25, Math.abs(start) * .15);
        let end = Math.min(parameter.max ?? Infinity, start + delta);
        if (end === start) end = Math.max(parameter.min ?? -Infinity, start - delta);
        PM.animate(owner, 'effect.value', [{ t: 0, v: start }, { t: 1, v: end }], { prop: effect.p[parameter.k] });
      }
    }
    if (owner.masks.length) PM.animate(owner, 'mask.x', [{ t: 0, v: -17 }, { t: 1, v: 19 }], { prop: owner.masks[0].p.x });
    if (fixture.paths) {
      const mask = owner.masks[0];
      PM.animate(owner, `mp.${mask.id}.v.${mask.path.vertices[0].id}.x`, [{ t: 0, v: -44 }, { t: 1, v: -23 }], { prop: mask.path.vertices[0].p.x });
      PM.animate(owner, `m.${mask.id}.rotation`, [{ t: 0, v: -13 }, { t: 1, v: 31 }], { prop: mask.p.rotation });
      PM.animate(owner, `m.${mask.id}.feather`, [{ t: 0, v: 3 }, { t: 1, v: 11 }], { prop: mask.p.feather });
    }
    await PM.prepareFrame(0);
    if (typeof PM.GL.createOpaqueReadback !== 'function') throw new Error('Opaque readback factory missing');
    const probe = PM.GL.createOpaqueReadback(128, 96), supported = !!probe; probe?.dispose();
    return supported;
  }, fixture);
}

for (const fixture of cases) test(`opaque asynchronous export preserves every pixel with ${fixture.name} at ${fixture.depth}-bit depth`, async ({ session }) => {
  await session.openEditor(); const supported = await effectProject(session, fixture);
  await session.app.evaluate(({ ipcMain }) => {
    const original = (ipcMain as any)._invokeHandlers.get('render:write');
    ipcMain.removeHandler('render:write'); ipcMain.handle('render:write', async (...args: any[]) => {
      const state = (globalThis as any).__effectWrites;
      state.maxActive = Math.max(state.maxActive, ++state.active);
      try {
        if (args[1].repeat !== undefined) throw new Error('Animated effects unexpectedly reused a frame');
        state.hash.update(args[1].data); state.bytes += args[1].data.length; state.writes++;
        return await original(...args);
      } finally { state.active--; }
    });
  });
  let referenceRaw: string | undefined, referenceDecoded: string | undefined;
  for (const optimized of [false, true]) {
    const output = path.join(session.userData, `effects-${optimized}.mp4`);
    await session.app.evaluate(({ dialog }, output) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      (globalThis as any).__effectWrites = { active: 0, maxActive: 0, bytes: 0, writes: 0,
        hash: process.getBuiltinModule('node:crypto').createHash('sha256') };
    }, output);
    const result = await session.page.evaluate(async optimized => {
      const PM = (window as any).PM, factory = PM.GL.createOpaqueReadback, pixels = PM.GL.renderToPixels;
      const targets = new Set<Uint8Array>(), buffers = new Set<ArrayBufferLike>();
      const state = { serial: 0, sessions: 0, activeSessions: 0, maxSessions: 0, begins: 0, reads: 0, pending: 0, maxPending: 0, disposed: 0 };
      PM.GL.renderToPixels = (...args: any[]) => { state.serial++; return pixels(...args); };
      PM.GL.createOpaqueReadback = optimized ? (...args: any[]) => {
        const readback = factory(...args); if (!readback) return readback;
        state.sessions++; state.maxSessions = Math.max(state.maxSessions, ++state.activeSessions);
        return {
          begin(...args: any[]) {
            const ticket = readback.begin(...args); state.begins++; state.maxPending = Math.max(state.maxPending, ++state.pending);
            let disposed = false;
            return { readInto(target: Uint8Array, cancelled: () => boolean) {
              targets.add(target); buffers.add(target.buffer); state.reads++; return ticket.readInto(target, cancelled);
            }, dispose() { if (!disposed) { disposed = true; state.pending--; } ticket.dispose(); } };
          }, dispose() { state.disposed++; state.activeSessions--; readback.dispose(); },
        };
      } : undefined;
      const before = PM.serialize(), time = PM.time, quality = PM.quality;
      try {
        const result = await PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4,
          range: 'all', quality: 'draft', audio: false, mblur: false });
        return { result, ...state, targets: targets.size, buffers: buffers.size,
          preserved: before === PM.serialize() && time === PM.time && quality === PM.quality };
      } finally { PM.GL.createOpaqueReadback = factory; PM.GL.renderToPixels = pixels; }
    }, optimized);
    expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true);
    expect(result).toMatchObject(optimized && supported
      ? { serial: 0, sessions: 1, activeSessions: 0, maxSessions: 1, begins: 4, reads: 4, pending: 0, maxPending: 1, disposed: 1, targets: 1, buffers: 1 }
      : { serial: 4, sessions: 0, activeSessions: 0, maxSessions: 0, begins: 0, reads: 0, pending: 0, maxPending: 0, disposed: 0, targets: 0, buffers: 0 });
    const main = await session.app.evaluate(() => {
      const state = (globalThis as any).__effectWrites;
      return { active: state.active, maxActive: state.maxActive, bytes: state.bytes, writes: state.writes, hash: state.hash.digest('hex') };
    });
    expect(main).toMatchObject({ active: 0, maxActive: 1, bytes: 128 * 96 * 4 * 4, writes: 4 });
    const decoded = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
      ['-v', 'error', '-i', output, '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'], { encoding: 'utf8' });
    const frames = decoded.split('\n').filter(line => line && !line.startsWith('#'));
    expect(frames).toHaveLength(4); expect(new Set(frames.map(line => line.split(',').at(-1))).size).toBeGreaterThan(1);
    const first = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
      ['-v', 'error', '-i', output, '-frames:v', '1', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
    expect(first.length).toBe(128 * 96 * 4);
    const colors = new Set<number>(); for (let at = 0; at < first.length; at += 4) colors.add(first.readUInt32LE(at));
    expect(colors.size).toBeGreaterThan(32);
    if (referenceRaw === undefined) { referenceRaw = main.hash; referenceDecoded = decoded; }
    else { expect(main.hash).toBe(referenceRaw); expect(decoded).toBe(referenceDecoded); }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const change of ['custom', 'builtin', 'default', 'revision', 'historical'] as const) test(`opaque export discards a queued frame after ${change === 'revision' ? 'a saved effect changes revision' : change === 'historical' ? 'a historical shader is replaced' : change === 'default' ? 'an effect default changes in place' : `the effect provider is replaced by ${change === 'builtin' ? 'the same builtin owner' : 'a custom owner'}`}`, async ({ session }) => {
  await session.openEditor(); const supported = await effectProject(session);
  test.skip(!supported, 'Asynchronous opaque readback unavailable');
  const expected = await session.page.evaluate(async change => {
    const PM = (window as any).PM;
    const source = PM.proj.layers[0].fx[0];
    if (change === 'revision' || change === 'historical') source.version = 1;
    const sameOwner = change === 'builtin' || change === 'historical';
    const defaultParameter = change === 'default' ? PM.Kernel.effects.get('blur').params[0] : undefined;
    const oldDefault = defaultParameter?.def;
    if (defaultParameter) {
      // Older projects can omit a channel and use the live provider default.
      delete PM.proj.layers[0].fx[0].p.amount; PM.touch();
    }
    const definition = sameOwner ? structuredClone(PM.Kernel.effects.get('blur')) : { id: 'blur', label: 'Replacement tint', group: 'Test', passes: 1, params: [], rawShader: true,
      frag: '#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nout vec4 o;\nvoid main(){vec4 c=texture(u_tex,v_st);o=vec4(vec3(.1,.8,.3)*c.a,c.a);}' };
    if (sameOwner) {
      const target = change === 'historical' ? definition.legacy[0] : definition;
      const output = change === 'historical' ? 'o = sum / max(wsum, 1e-4);' : 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));';
      if (!target.frag.includes(output)) throw new Error('Builtin blur output changed');
      target.frag = target.frag.replace(output, change === 'historical'
        ? 'o=sum/max(wsum,1e-4);o.rgb*=vec3(.1,.8,.3);'
        : 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));o.rgb*=vec3(.1,.8,.3);');
    }
    const owner = sameOwner ? 'effects-basic' : 'e2e-replacement';
    let registration: any;
    const replace = () => {
      if (change === 'revision') source.version = 2;
      else if (defaultParameter) defaultParameter.def = 89;
      else registration = PM.Kernel.registerEffect(owner, definition);
    };
    const restore = () => {
      if (change === 'revision') source.version = 1;
      else if (defaultParameter) defaultParameter.def = oldDefault;
      else registration?.dispose();
    };
    await PM.prepareFrame(.25);
    const oldSecond = PM.GL.renderToPixels(.25, 128, 96,
      { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
    const reference: number[] = [];
    try {
      for (let frame = 0; frame < 4; frame++) {
        if (frame === 1) replace();
        await PM.prepareFrame(frame / 4);
        const pixels = PM.GL.renderToPixels(frame / 4, 128, 96,
          { opaque: true, topDownOpaque: true, mblur: false, mbSamples: 1, shutter: .5 });
        if (frame === 1 && pixels.every((byte: number, index: number) => byte === oldSecond[index]))
          throw new Error('Effect change must visibly change the same timestamp');
        reference.push(...pixels);
      }
    } finally { restore(); await PM.prepareFrame(0); }
    (window as any).__replaceOpaqueEffect = replace;
    (window as any).__disposeOpaqueEffect = restore;
    return reference;
  }, change);
  try {
    await session.app.evaluate(({ ipcMain, dialog }, output) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      const state = { held: false, chunks: [] as number[][] };
      (globalThis as any).__heldOpaqueEffect = state;
      const original = (ipcMain as any)._invokeHandlers.get('render:write');
      ipcMain.removeHandler('render:write'); ipcMain.handle('render:write', async (...args: any[]) => {
        state.chunks.push(Array.from(args[1].data)); const result = await original(...args);
        if (state.chunks.length === 1) {
          state.held = true; await new Promise<void>(resolve => { (globalThis as any).__releaseOpaqueEffect = resolve; }); state.held = false;
        }
        return result;
      });
    }, path.join(session.userData, 'effect-replacement.mp4'));
    await session.page.evaluate(() => {
      const PM = (window as any).PM, factory = PM.GL.createOpaqueReadback;
      const state = { begins: [] as number[], reads: [] as number[], disposed: [] as number[], active: 0 };
      (window as any).__queuedOpaqueEffect = state;
      PM.GL.createOpaqueReadback = (...args: any[]) => {
        const readback = factory(...args); if (!readback) return readback; state.active++;
        return {
          begin(...args: any[]) {
            const time = args[0], ticket = readback.begin(...args); state.begins.push(time);
            return { readInto(...args: any[]) { state.reads.push(time); return ticket.readInto(...args); },
              dispose() { state.disposed.push(time); ticket.dispose(); } };
          }, dispose() { state.active--; readback.dispose(); },
        };
      };
      (window as any).__restoreOpaqueFactory = () => { PM.GL.createOpaqueReadback = factory; };
      (window as any).__opaqueEffectJob = PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 4,
        range: 'all', quality: 'draft', audio: false, mblur: false });
    });
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__heldOpaqueEffect.held)).toBe(true);
    await expect.poll(() => session.page.evaluate(() => (window as any).__queuedOpaqueEffect.begins)).toEqual([0, .25]);
    await session.page.evaluate(() => (window as any).__replaceOpaqueEffect());
    await session.app.evaluate(() => (globalThis as any).__releaseOpaqueEffect());
    expect(await session.page.evaluate(() => (window as any).__opaqueEffectJob)).toEqual({ cancelled: false });
    expect(await session.app.evaluate(() => (globalThis as any).__heldOpaqueEffect.chunks.flat())).toEqual(expected);
    const captured = await session.page.evaluate(() => (window as any).__queuedOpaqueEffect);
    expect(captured).toEqual(change !== 'custom'
      ? { begins: [0, .25, .5, .75], reads: [0, .5, .75], disposed: [0, .25, .5, .75], active: 0 }
      : { begins: [0, .25], reads: [0], disposed: [0, .25], active: 0 });
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseOpaqueEffect?.());
    await session.page.evaluate(async () => {
      await (window as any).__opaqueEffectJob; (window as any).__restoreOpaqueFactory?.(); (window as any).__disposeOpaqueEffect?.();
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
