import { expect, test } from './helpers/app';

test('recoverable blank text is retried before its GPU source or full preview is retained', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, GL = PM.GL, gl = GL.gl;
    PM.pause(); PM.agentFrameCapture = true; PM.perf.auto = false;
    await document.fonts.ready;
    PM.proj = PM.mkProject({ name: 'Recoverable text source', w: 128, h: 72, dur: 2 });
    const layer = PM.mkLayer('text', { d: { text: 'VISIBLE', size: 20, color: '#ffffff' },
      p: { 'position.x': 12, 'position.y': 32 } }, PM.proj);
    PM.proj.layers = [layer]; PM.touch(); GL.resize(128, 72, null);
    GL.render(0, { mblur: false, exporting: true });
    PM.rasterClear(); PM.agentFrameCapture = false;
    const raster = PM.raster;
    const litPixels = () => {
      const bytes = new Uint8Array(128 * 72 * 4);
      gl.readPixels(0, 0, 128, 72, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
      let lit = 0;
      for (let i = 0; i < bytes.length; i += 4) if (bytes[i]! > 10) lit++;
      return lit;
    };
    try {
      PM.raster = (...args: any[]) => {
        const source = raster(...args), blank = document.createElement('canvas');
        blank.width = source.cv.width; blank.height = source.cv.height;
        return { ...source, cv: blank, blank: true };
      };
      GL.render(.5, { mblur: false });
      const blankPixels = litPixels(), retainedBlank = GL.previewFrames.count;
      PM.raster = raster;
      // Same time, source key and document revision: only source readiness changed.
      GL.render(.5, { mblur: false });
      const recoveredPixels = litPixels(), recoveryHit = GL.stats.previewHit;
      GL.render(.5, { mblur: false });
      return { blankPixels, retainedBlank, recoveredPixels, recoveryHit, replayHit: GL.stats.previewHit };
    } finally { PM.raster = raster; }
  });
  expect(result.blankPixels).toBe(0);
  expect(result.retainedBlank).toBe(0);
  expect(result.recoveredPixels).toBeGreaterThan(20);
  expect(result.recoveryHit).toBe(false);
  expect(result.replayHit).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a full preview cache reuses GPU targets while scrubbing through new frames', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, GL = PM.GL, gl = GL.gl;
    PM.pause(); PM.agentFrameCapture = true; PM.perf.auto = false;
    await document.fonts.ready;
    PM.proj = PM.mkProject({ name: 'Preview target reuse', w: 128, h: 72, dur: 8 });
    PM.proj.layers = [PM.mkLayer('shape', { d: { w: 48, h: 32, color: '#ed5b32' } }, PM.proj)];
    PM.animate(PM.proj.layers[0], 'rotation', [{ t: 0, v: 0 }, { t: 8, v: 180 }]);
    PM.touch(); GL.resize(128, 72, null);
    GL.render(0, { mblur: false, exporting: true });
    PM.Memory.setBudget('preview', 1024 * 1024);
    PM.agentFrameCapture = false;
    // Fill the cache and allow both working render targets to settle.
    for (let i = 0; i < 40; i++) GL.render(i / 30, { mblur: false });
    const create = gl.createTexture.bind(gl), dispose = gl.deleteTexture.bind(gl);
    let allocations = 0, deletions = 0;
    gl.createTexture = () => { allocations++; return create(); };
    gl.deleteTexture = (texture: WebGLTexture) => { deletions++; return dispose(texture); };
    try {
      for (let i = 40; i < 100; i++) GL.render(i / 30, { mblur: false });
      return { allocations, deletions, preview: PM.Memory.stats().preview, error: gl.getError() };
    } finally { gl.createTexture = create; gl.deleteTexture = dispose; }
  });
  expect(result.allocations).toBe(0);
  expect(result.deletions).toBe(0);
  expect(result.preview.bytes).toBeGreaterThan(0);
  expect(result.preview.bytes).toBeLessThanOrEqual(result.preview.budget);
  expect(result.error).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('remembered previews preserve pixels, skip rendering, obey the cap, and refresh after changes', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, GL = PM.GL, gl = GL.gl;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    await document.fonts.ready;
    PM.proj = PM.mkProject({ name: 'Preview memory test', w: 128, h: 72, dur: 2 });
    const layer = PM.mkLayer('shape', { d: { w: 48, h: 32, color: '#ed5b32' }, p: { 'position.x': 64, 'position.y': 36 } }, PM.proj);
    PM.proj.layers = [layer];
    PM.animate(layer, 'rotation', [{ t: 0, v: 0 }, { t: 1, v: 90 }]);
    PM.touch(); GL.resize(128, 72, null);
    const options = { mblur: false };
    const pixels = () => {
      const bytes = new Uint8Array(GL.canvas.width * GL.canvas.height * 4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.readPixels(0, 0, GL.canvas.width, GL.canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
      return Array.from(bytes);
    };
    // Compile using the uncached path so compilation cannot obscure the check.
    GL.render(0, { ...options, exporting: true });
    GL.render(0, options); const first = pixels();
    GL.render(.5, options); const middle = pixels();
    const renderProject = GL.renderProject;
    let renderCalls = 0;
    GL.renderProject = (...args: any[]) => { renderCalls++; return renderProject(...args); };
    GL.render(0, options); const replay = pixels();
    const replayCalls = renderCalls, replayHit = GL.stats.previewHit;
    const bytesBefore = PM.Memory.stats().preview.bytes;
    layer.d.color = '#32abed'; PM.touch();
    GL.render(0, options); const edited = pixels(), editHit = GL.stats.previewHit;
    GL.resize(64, 36, null); GL.render(0, options); const resizeHit = GL.stats.previewHit;
    GL.resize(128, 72, null);
    PM.Memory.setBudget('preview', 1024 * 1024);
    for (let i = 0; i < 60; i++) GL.render(i / 30, options);
    const limited = PM.Memory.stats().preview;
    PM.Memory.pressure('critical'); const pressureBytes = PM.Memory.stats().preview.bytes;
    PM.Memory.setBudget('preview', 0); const offBytes = PM.Memory.stats().preview.bytes;
    GL.render(0, options); GL.render(0, options); const offHit = GL.stats.previewHit;
    GL.renderProject = renderProject;
    return { first, middle, replay, edited, replayCalls, replayHit, editHit, resizeHit, bytesBefore, limited, pressureBytes, offBytes, offHit, glError: gl.getError() };
  });
  expect(result.replay).toEqual(result.first);
  expect(result.middle).not.toEqual(result.first);
  expect(result.replayCalls).toBe(0);
  expect(result.replayHit).toBe(true);
  expect(result.bytesBefore).toBeGreaterThan(0);
  expect(result.edited).not.toEqual(result.first);
  expect(result.editHit).toBe(false);
  expect(result.resizeHit).toBe(false);
  expect(result.limited.bytes).toBeGreaterThan(0);
  expect(result.limited.bytes).toBeLessThanOrEqual(result.limited.budget);
  expect(result.pressureBytes).toBeLessThanOrEqual(256 * 1024);
  expect(result.offBytes).toBe(0);
  expect(result.offHit).toBe(false);
  expect(result.glError).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('preview memory limit is available in Settings and survives restarting the app', async ({ session }) => {
  await session.page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
  const limit = session.page.locator('select[aria-label="Preview memory limit"]');
  await expect(limit).toHaveValue('256');
  await limit.selectOption('512');
  expect(await session.page.evaluate(() => (window as any).PM.Memory.budget('preview'))).toBe(512 * 1024 * 1024);
  await session.page.evaluate(() => (window as any).PM.SettingsUI.close());
  await session.relaunch();
  await session.page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
  await expect(session.page.locator('select[aria-label="Preview memory limit"]')).toHaveValue('512');
  await session.page.locator('select[aria-label="Preview memory limit"]').selectOption('0');
  expect(await session.page.evaluate(() => (window as any).PM.Memory.stats().preview.bytes)).toBe(0);
  await session.page.screenshot({ path: '/tmp/powermove-preview-memory-settings.png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('ordinary playback and backward scrubbing automatically reuse remembered frames', async ({ session }) => {
  await session.openEditor();
  if (await session.page.evaluate(() => Boolean((window as any).PM.EngineConnection))) {
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  }
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    await document.fonts.ready;
    PM.proj = PM.mkProject({ name: 'Loop memory', w: 160, h: 90, dur: 1, fps: 10 });
    PM.proj.work = [0, .3]; PM.loop = true;
    PM.proj.layers = [PM.mkLayer('shape', { d: { w: 48, h: 32, color: '#ed5b32' } }, PM.proj)];
    PM.touch(); PM.bus.emit('project');
    (window as any).__previewHits = [];
    if (PM.EngineConnection?.status().route === 'engine') {
      // The engine draws the viewer: each presented frame reports whether it
      // came from the engine's remembered-frame memory cache.
      PM.bus.on('preview:presented', (frame: any) => {
        if (PM.EngineConnection.status().presentedCached) (window as any).__previewHits.push({ time: frame.time, playing: PM.playing });
      });
    } else {
      const render = PM.GL.render;
      PM.GL.render = (time: number, opt: any) => {
        const result = render(time, opt);
        if (PM.GL.stats.previewHit) (window as any).__previewHits.push({ time, playing: PM.playing });
        return result;
      };
    }
    PM.setTime(0); PM.play();
  });
  await expect.poll(() => session.page.evaluate(() => (window as any).__previewHits.filter((hit: any) => hit.playing).length)).toBeGreaterThan(2);
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); (window as any).__previewHits = []; PM.setTime(0, { force: true });
  });
  await expect.poll(() => session.page.evaluate(() => (window as any).__previewHits.filter((hit: any) => !hit.playing && hit.time === 0).length)).toBeGreaterThan(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
