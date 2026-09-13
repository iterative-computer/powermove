import { expect, test } from './helpers/app';

for (const paused of [false, true]) {
test(`prepares a dense group entrance while ${paused ? 'paused just before it' : 'playing'}`, async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  await page.evaluate(paused => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Dense entrance', w: 640, h: 360, dur: 10, fps: 30 });
    p.layers = Array.from({ length: 200 }, (_, i) => PM.mkLayer('shape', {
      name: `Tile ${i}`, from: 4, dur: 5,
      d: { w: 14, h: 14, color: '#' + (0x100000 + i * 151).toString(16), radius: 2 },
      p: { 'position.x': 20 + i % 20 * 30, 'position.y': 20 + Math.floor(i / 20) * 30 },
    }, p));
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    PM.ProjectsScreen.hide(); PM.perf.auto = false; PM.quality = 1;
    (window as any).warmCalls = 0;
    const raster = PM.raster;
    PM.raster = (...args: any[]) => { if (args[2] === 4 && PM.time < 4) (window as any).warmCalls++; return raster(...args); };
    PM.setTime(paused ? 119 / 30 : 2.1);
    if (!paused) PM.play();
  }, paused);
  await expect.poll(() => page.evaluate(() => (window as any).warmCalls), { timeout: 1800 }).toBeGreaterThanOrEqual(200);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const gl = PM.GL.gl, upload = gl.texImage2D.bind(gl);
    let uploads = 0;
    gl.texImage2D = (...args: any[]) => { uploads++; return upload(...args); };
    PM.GL.render(4, { mblur: false });
    const first = new Uint8Array(gl.drawingBufferWidth * gl.drawingBufferHeight * 4);
    gl.readPixels(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, gl.RGBA, gl.UNSIGNED_BYTE, first);
    const entranceUploads = uploads;
    gl.texImage2D = upload;
    PM.GL.render(4, { mblur: false, sourceClipping: false });
    const reference = new Uint8Array(first.length);
    gl.readPixels(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, gl.RGBA, gl.UNSIGNED_BYTE, reference);
    let maxDifference = 0;
    for (let i = 0; i < first.length; i++) maxDifference = Math.max(maxDifference, Math.abs(first[i]! - reference[i]!));
    return { entranceUploads, maxDifference, draws: PM.GL.stats.draws };
  });
  expect(result.entranceUploads).toBe(0);
  expect(result.maxDifference).toBeLessThanOrEqual(1);
  expect(result.draws).toBeGreaterThanOrEqual(200);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

}

test('fitted previews skip offscreen oversized sources and keep the rendered pixels', async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Clipped fitted preview', w: 320, h: 180, dur: 5 });
    p.layers = [
      PM.mkLayer('shape', { d: { w: 1500, h: 800, color: '#5274FF', radius: 20 }, p: { 'position.x': 800, 'position.y': 500 } }, p),
      PM.mkLayer('shape', { d: { w: 5000, h: 5000, color: '#FF4455' }, p: { 'position.x': -6000, 'position.y': -6000 } }, p),
    ];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    PM.pause(); PM.agentFrameCapture = true; PM.quality = 1;
    PM.GL.previewViewport = null; PM.GL.resize(320, 180);
    const gl = PM.GL.gl;
    const pixels = (sourceClipping: boolean) => {
      PM.GL.render(0, { mblur: false, sourceClipping });
      const data = new Uint8Array(320 * 180 * 4);
      gl.readPixels(0, 0, 320, 180, gl.RGBA, gl.UNSIGNED_BYTE, data);
      return { data, draws: PM.GL.stats.draws, bytes: PM.GL.memoryStats().textures.bytes };
    };
    PM.rasterClear();
    const clipped = pixels(true), reference = pixels(false);
    let maxDifference = 0;
    for (let i = 0; i < clipped.data.length; i++) maxDifference = Math.max(maxDifference, Math.abs(clipped.data[i]! - reference.data[i]!));
    return { maxDifference, clippedDraws: clipped.draws, referenceDraws: reference.draws, clippedBytes: clipped.bytes, referenceBytes: reference.bytes };
  });
  expect(result.maxDifference).toBeLessThanOrEqual(1);
  expect(result.clippedDraws).toBeLessThan(result.referenceDraws);
  expect(result.clippedBytes).toBeLessThan(result.referenceBytes / 4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('group effects preserve pixels while clipping oversized child sources', async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Large blurred group', w: 320, h: 180, dur: 5 });
    const group = PM.mkLayer('group', {}, p);
    p.layers = [
      group,
      PM.mkLayer('shape', { d: { w: 5000, h: 140, color: '#5274FF', radius: 20 }, p: { 'position.x': 80, 'position.y': 100 } }, p),
      PM.mkLayer('shape', { d: { w: 1200, h: 1000, color: '#FF4455', radius: 100 }, p: { 'position.x': -900, 'position.y': -900 } }, p),
      PM.mkLayer('text', { d: { text: 'Outside the group target', size: 120 }, p: { 'position.x': 3000, 'position.y': 100 } }, p),
    ];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    for (const child of PM.proj.layers.filter((l: any) => l.id !== group.id)) child.group = group.id;
    PM.touch();
    const add = PM.Edit.apply({ type: 'add_effect', target: group.id, effect: 'blur', parameters: { amount: 24 } }, { origin: 'agent' });
    if (!add.ok) throw new Error('Expected a renderable group blur');
    PM.pause(); PM.agentFrameCapture = true; PM.quality = 1;
    PM.GL.previewViewport = null; PM.GL.resize(320, 180);
    const gl = PM.GL.gl;
    const render = (sourceClipping: boolean) => {
      PM.rasterClear();
      PM.GL.render(0, { mblur: false, sourceClipping, exporting: true });
      // Compile the effect synchronously first; compare the regular preview paths.
      PM.rasterClear();
      PM.GL.render(0, { mblur: false, sourceClipping });
      const data = new Uint8Array(320 * 180 * 4);
      gl.readPixels(0, 0, 320, 180, gl.RGBA, gl.UNSIGNED_BYTE, data);
      return { data, bytes: PM.GL.memoryStats().textures.bytes, passes: PM.GL.stats.passes };
    };
    const clipped = render(true), reference = render(false);
    let maxDifference = 0;
    for (let i = 0; i < clipped.data.length; i++) maxDifference = Math.max(maxDifference, Math.abs(clipped.data[i]! - reference.data[i]!));
    return { maxDifference, clippedBytes: clipped.bytes, referenceBytes: reference.bytes, passes: clipped.passes };
  });
  expect(result.maxDifference).toBeLessThanOrEqual(1);
  expect(result.passes).toBeGreaterThan(0);
  expect(result.clippedBytes).toBeLessThan(result.referenceBytes / 4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
