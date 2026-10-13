import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

test('idle preparation leaves the canvas and playhead alone, then normal Play reuses accurate frames', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); await document.fonts.ready;
    PM.proj = PM.mkProject({ name: 'Automatic preview', w: 640, h: 360, fps: 24, dur: 1 });
    PM.proj.work = [0, .5];
    const layer = PM.mkLayer('shape', { d: { w: 120, h: 80, color: '#ed5b32' },
      p: { 'position.x': 320, 'position.y': 180 } }, PM.proj);
    PM.proj.layers = [layer];
    PM.animate(layer, 'rotation', [{ t: 0, v: 0 }, { t: 1, v: 90 }]);
    PM.touch(); PM.bus.emit('project'); PM.setPreviewResolution('0.25'); PM.setTime(0);
    const gl = PM.GL.gl;
    (window as any).__readPreview = () => {
      const pixels = new Uint8Array(PM.GL.canvas.width * PM.GL.canvas.height * 4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.readPixels(0, 0, PM.GL.canvas.width, PM.GL.canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      return Array.from(pixels);
    };
    PM.GL.render(0, { mblur: true, mbSamples: 12, shutter: .5, hideShy: false });
    (window as any).__idleFirst = (window as any).__readPreview();
  });
  await expect.poll(() => session.page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.GL.hasPreviewFrame(11 / 24, { mblur: true, mbSamples: 6, shutter: .5, hideShy: false });
  }), { timeout: 20_000 }).toBe(true);
  const idle = await session.page.evaluate(() => ({ time: (window as any).PM.time,
    before: (window as any).__idleFirst, after: (window as any).__readPreview(),
    memory: (window as any).PM.Memory.stats().preview,
  }));
  expect(idle.time).toBe(0); expect(idle.after).toEqual(idle.before);
  expect(idle.memory.bytes).toBeLessThanOrEqual(idle.memory.budget);
  const parity = await session.page.evaluate(() => {
    const PM = (window as any).PM, options = { mblur: true, mbSamples: 6, shutter: .5, hideShy: false };
    PM.AutomaticPreview.cancel(); PM.agentFrameCapture = true;
    PM.GL.render(7 / 24, options); const direct = (window as any).__readPreview();
    PM.agentFrameCapture = false;
    PM.GL.render(7 / 24, options);
    const prepared = (window as any).__readPreview(), hit = PM.GL.stats.previewHit;
    return { direct, prepared, hit };
  });
  expect(parity.hit).toBe(true); expect(parity.prepared).toEqual(parity.direct);
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    (window as any).__automaticHits = [];
    const render = PM.GL.render;
    PM.GL.render = (...args: any[]) => {
      const value = render(...args);
      if (PM.playing) (window as any).__automaticHits.push({ hit: !!PM.GL.stats.previewHit, time: args[0], at: performance.now() });
      return value;
    };
    PM.setTime(0); PM.loop = true; PM.play();
  });
  await expect.poll(() => session.page.evaluate(() => (window as any).__automaticHits.length)).toBeGreaterThan(24);
  const playback = await session.page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    return { frames: (window as any).__automaticHits, error: PM.GL.gl.getError() };
  });
  expect(playback.frames.every((frame: any) => frame.hit)).toBe(true);
  console.log('PREPARED_PLAYBACK', JSON.stringify({ frames: playback.frames.length,
    fps: (playback.frames.length - 1) * 1000 / (playback.frames.at(-1).at - playback.frames[0].at),
    cacheHits: playback.frames.filter((frame: any) => frame.hit).length }));
  expect(playback.error).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('prepared display frames preserve 8, 16 and 32-bit HDR and translucent output', async ({ session }) => {
  await session.openEditor();
  const results = await session.page.evaluate(async () => {
    const PM = (window as any).PM, GL = PM.GL, gl = GL.gl;
    PM.pause(); PM.AutomaticPreview.cancel(); PM.interactionActive = () => true;
    await document.fonts.ready;
    const read = () => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      const pixels = new Uint8Array(GL.canvas.width * GL.canvas.height * 4);
      gl.readPixels(0, 0, GL.canvas.width, GL.canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      return Array.from(pixels);
    };
    const results = [];
    for (const depth of GL.supportedColorDepths) {
      PM.proj = PM.mkProject({ name: 'Prepared color parity', w: 128, h: 72, fps: 24, dur: 2, colorDepth: depth });
      PM.proj.colorDepth = depth;
      const shape = PM.mkLayer('shape', { d: { w: 80, h: 40, color: '#ec7347' },
        p: { 'position.x': 64, 'position.y': 36, opacity: 43 } }, PM.proj);
      const shader = PM.mkLayer('shader', { d: { w: 128, h: 72,
        code: 'void main(){fragColor=vec4(1.6+uv.x,0.17+iTime*0.03,0.07,0.6);}', uniforms: {} } }, PM.proj);
      PM.proj.layers = [shape, shader]; PM.touch(); GL.resize(128, 72, null);
      const options = { mblur: true, mbSamples: 6, shutter: .5, hideShy: false };
      GL.render(.25, { ...options, exporting: true }); const original = read();
      const oldTime = PM.time;
      const prepared = await GL.preparePreviewFrame(.25, options, new Map(), () => true);
      const unchanged = read(); GL.render(.25, options);
      results.push({ depth, prepared, original, unchanged, cached: read(), hit: GL.stats.previewHit,
        timeUnchanged: PM.time === oldTime, memory: PM.Memory.stats().preview, error: gl.getError() });
    }
    return results;
  });
  expect(results.length).toBeGreaterThan(0);
  for (const result of results) {
    expect(result.prepared).toBe(true); expect(result.hit).toBe(true);
    expect(result.unchanged).toEqual(result.original); expect(result.cached).toEqual(result.original);
    expect(result.timeUnchanged).toBe(true); expect(result.error).toBe(0);
    expect(result.memory.bytes).toBe(128 * 72 * 4);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('edits, shader replacement and memory Off reject pending frames and do not retain scratch targets', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, GL = PM.GL, gl = GL.gl;
    PM.pause(); PM.AutomaticPreview.cancel(); PM.interactionActive = () => true;
    PM.proj = PM.mkProject({ name: 'Cancelled automatic frames', w: 128, h: 72, fps: 24, dur: 2 });
    PM.proj.layers = [PM.mkLayer('shape', { d: { w: 32, h: 20, color: '#f75522' } }, PM.proj)];
    PM.touch(); GL.resize(128, 72, null); await document.fonts.ready;
    const options = { mblur: true, mbSamples: 6, shutter: .5, hideShy: false };
    GL.render(0, { ...options, exporting: true });
    const wait = gl.clientWaitSync.bind(gl);
    gl.clientWaitSync = () => gl.TIMEOUT_EXPIRED;
    const results = [];
    try {
      for (const change of ['edit', 'shader', 'off']) {
        const job = GL.preparePreviewFrame(.25, options, new Map(), () => true);
        if (change === 'edit') { PM.touch(); PM.bus.emit('layers'); }
        else if (change === 'shader') GL.dropProgram('fx:blur');
        else PM.Memory.setBudget('preview', 0);
        results.push(await job);
      }
    } finally { gl.clientWaitSync = wait; }
    return { results, bytes: PM.Memory.stats().preview.bytes, pending: GL.previewPreparationBytes,
      busy: GL.pool.filter((frame: any) => frame.busy).length, error: gl.getError() };
  });
  expect(result.results).toEqual([false, false, false]);
  expect(result.bytes).toBe(0); expect(result.pending).toBe(0);
  expect(result.busy).toBeLessThanOrEqual(1); expect(result.error).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const matte of [false, true]) test(`automatic video preparation preserves live decoders and matches ${matte ? 'hidden video mattes' : 'cropped alpha frames'}`, async ({ session }) => {
  await session.openEditor();
  const source = path.join(session.userData, 'automatic-alpha.webm');
  const raw = Buffer.concat(Array.from({ length: 20 }, (_, frame) => {
    const bytes = Buffer.alloc(64 * 64 * 4);
    for (let y = 0; y < 64; y++) for (let x = 0; x < 64; x++) {
      const at = (y * 64 + x) * 4;
      bytes[at] = (x * 5 + frame * 3) % 256; bytes[at + 1] = (y * 7 + frame) % 256;
      bytes[at + 2] = (x * 3 + y * 2 + 80) % 256;
      bytes[at + 3] = x < 21 ? 0 : x > 42 ? 255 : 128;
    }
    return bytes;
  }));
  execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), ['-v', 'error', '-f', 'rawvideo',
    '-pix_fmt', 'rgba', '-s', '64x64', '-r', '10', '-i', 'pipe:0', '-c:v', 'libvpx-vp9',
    '-lossless', '1', '-g', '1', '-pix_fmt', 'yuva420p', '-auto-alt-ref', '0', source], { input: raw });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AutomaticPreview.cancel();
    PM.proj = PM.mkProject({ name: 'Automatic private alpha', w: 64, h: 64, fps: 10, dur: 2 });
    PM.proj.work = [0, .8]; PM.bus.emit('project');
    const input = document.createElement('input'); input.type = 'file'; input.id = 'automatic-source'; document.body.append(input);
  });
  await session.page.locator('#automatic-source').setInputFiles(source);
  await session.page.evaluate(async matte => {
    const PM = (window as any).PM;
    await PM.importFiles([(document.querySelector('#automatic-source') as HTMLInputElement).files![0]]);
    const layer = PM.proj.layers.find((item: any) => item.type === 'video');
    layer.from = 0; layer.dur = 2; layer.d.trim = PM.P(0); layer.d.speed = PM.P(1);
    layer.d.w = PM.P(90); layer.d.h = PM.P(70); layer.d.fit = 'cover';
    layer.p['position.x'].v = 32; layer.p['position.y'].v = 32;
    const asset = PM.assets.get(layer.d.asset); asset.w = 128; asset.h = 64;
    if (matte) {
      const shape = PM.mkLayer('shape', { d: { w: 64, h: 64, color: '#ed7242' },
        p: { 'position.x': 32, 'position.y': 32 } }, PM.proj);
      shape.matteSource = layer.id; layer.on = false;
      PM.proj.layers = [shape, layer];
    }
    (window as any).__automaticAsset = asset;
    PM.touch(); PM.bus.emit('layers'); PM.setPreviewResolution('1'); PM.setTime(0);
  }, matte);
  await session.page.waitForFunction(() => {
    const asset = (window as any).__automaticAsset;
    return !asset.el.seeking && asset.el.readyState >= 2;
  });
  const before = await session.page.evaluate(() => (window as any).__automaticAsset.el.currentTime);
  await expect.poll(() => session.page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.GL.hasPreviewFrame(.5, { mblur: true, mbSamples: 6, shutter: .5, hideShy: false });
  }), { timeout: 20_000 }).toBe(true);
  const after = await session.page.evaluate(() => (window as any).__automaticAsset.el.currentTime);
  expect(after).toBe(before);
  const prepared = await session.page.evaluate(() => {
    const PM = (window as any).PM; PM.AutomaticPreview.cancel(); PM.interactionActive = () => true;
    PM.GL.render(.5, { mblur: true, mbSamples: 6, shutter: .5, hideShy: false });
    const pixels = new Uint8Array(64 * 64 * 4), gl = PM.GL.gl;
    gl.readPixels(0, 0, 64, 64, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    const hit = PM.GL.stats.previewHit;
    PM.setTime(.5, { raw: true, force: true });
    // The ordinary clip list excludes a disabled video used as a matte. Force
    // its foreground reference pass to request the source seek before waiting.
    PM.agentFrameCapture = true;
    PM.GL.render(.5, { mblur: true, mbSamples: 6, shutter: .5, hideShy: false, previewReuse: false });
    PM.agentFrameCapture = false;
    return { pixels: Array.from(pixels), hit };
  });
  expect(prepared.hit).toBe(true);
  await session.page.waitForFunction(() => {
    const PM = (window as any).PM, asset = (window as any).__automaticAsset;
    return !asset.el.seeking && asset.el.readyState >= 2 && Math.abs(asset.el.currentTime - .5) < .01;
  });
  const direct = await session.page.evaluate(() => {
    const PM = (window as any).PM, gl = PM.GL.gl;
    PM.agentFrameCapture = true;
    PM.GL.render(.5, { mblur: true, mbSamples: 6, shutter: .5, hideShy: false });
    PM.agentFrameCapture = false;
    const pixels = new Uint8Array(64 * 64 * 4);
    gl.readPixels(0, 0, 64, 64, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    return { pixels: Array.from(pixels), error: gl.getError(), privateTextures: [...PM.GL.texes.keys()]
      .filter((key: string) => key.startsWith('idle-video:')).length, staging: PM.AutomaticPreview.stagingBytes };
  });
  expect(direct.pixels).toEqual(prepared.pixels);
  expect(direct.error).toBe(0); expect(direct.privateTextures).toBe(0); expect(direct.staging).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
