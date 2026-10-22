import { expect, test } from './helpers/app';

test('prepares editable paths for a held zoom cut without allocating huge background bitmaps', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Path zoom cut', w: 640, h: 360, dur: 8, fps: 30 });
    const camera = PM.mkLayer('null', {}, p);
    for (const channel of ['scale.x', 'scale.y']) camera.p[channel].kf = [
      { ...PM.KF(0, 150, 'linear'), outInterp: 'hold', hold: true }, PM.KF(4, 100, 'linear'),
    ];
    const path = (size: number, color: string) => ({ id: PM.uid('path'), parent: null, name: 'Rectangle',
      p: Object.fromEntries(Object.entries({ x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100,
        closed: true, fill: color, fillEnabled: true, fillOpacity: 100, stroke: color, strokeWidth: 0,
        strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1,
        repeatX: 0, repeatY: 0, repeatRotation: 0 }).map(([key, value]) => [key, PM.P(value)])),
      vertices: [[-size, -size], [size, -size], [size, size], [-size, size]].map(([x, y]) => ({ id: PM.uid('v'),
        p: Object.fromEntries(Object.entries({ x, y, inX: 0, inY: 0, outX: 0, outY: 0 }).map(([key, value]) => [key, PM.P(value)])) })),
    });
    const background = PM.mkLayer('shape', { parent: camera.id, d: { paths: [path(4000, '#232323')] } }, p);
    const tiles = Array.from({ length: 24 }, (_, i) => PM.mkLayer('shape', {
      parent: camera.id, d: { paths: [path(8, '#' + (0x200000 + i * 4000).toString(16))] },
      p: { 'position.x': 25 + i % 8 * 70, 'position.y': 25 + Math.floor(i / 8) * 70 },
    }, p));
    for (const layer of [...tiles, background]) layer.parent = camera.id;
    p.layers = [camera, ...tiles, background];
    // Keep independent card-thumbnail renders from warming the fixture at a
    // different resolution before this source-preparation check.
    PM.interactionActive = () => true;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    PM.ProjectsScreen.hide(); PM.perf.auto = false; PM.quality = 1; PM.pause();
    const viewer = PM.Kernel.services.get('viewer'); viewer.fit = false; viewer.zoom = .618; viewer.pan = [0, 0]; viewer.layout();
    (window as any).cutWarmLayers = new Set();
    const raster = PM.raster;
    PM.raster = (...args: any[]) => {
      if (args[2] === 4 && PM.time < 4 && args[5] === 4 * 1024 * 1024) (window as any).cutWarmLayers.add(args[0].id);
      return raster(...args);
    };
    PM.setTime(3);
  });
  await expect.poll(() => page.evaluate(() => (window as any).cutWarmLayers.size), { timeout: 3000 }).toBe(25);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.agentFrameCapture = true;
    const gl = PM.GL.gl, upload = gl.texImage2D;
    let uploads = 0;
    gl.texImage2D = function (...args: any[]) { if (args.length === 6) uploads++; return upload.apply(this, args); };
    const read = (sourceClipping: boolean) => {
      PM.GL.render(4, { mblur: false, sourceClipping });
      const pixels = new Uint8Array(gl.drawingBufferWidth * gl.drawingBufferHeight * 4);
      gl.readPixels(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      return pixels;
    };
    const first = read(true), entranceUploads = uploads;
    gl.texImage2D = upload;
    const bytes = PM.GL.memoryStats().textures.bytes;
    const reference = read(false);
    let maxDifference = 0;
    for (let i = 0; i < first.length; i++) maxDifference = Math.max(maxDifference, Math.abs(first[i]! - reference[i]!));
    return { entranceUploads, bytes, maxDifference, time: PM.time };
  });
  expect(result.entranceUploads).toBe(0);
  expect(result.bytes).toBeLessThan(16 * 1024 * 1024);
  expect(result.maxDifference).toBeLessThanOrEqual(1);
  expect(result.time).toBe(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const threeD of [false, true]) {
for (const paused of [false, true]) {
test(`prepares a dense ${threeD ? '3D' : '2D'} group entrance while ${paused ? 'paused just before it' : 'playing'}`, async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  await page.evaluate(({paused, threeD}) => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Dense entrance', w: 640, h: 360, dur: 10, fps: 30 });
    p.layers = Array.from({ length: 200 }, (_, i) => PM.mkLayer('shape', {
      name: `Tile ${i}`, from: 4, dur: 5,
      d: { w: 14, h: 14, color: '#' + (0x100000 + i * 151).toString(16), radius: 2 },
      p: { 'position.x': 20 + i % 20 * 30, 'position.y': 20 + Math.floor(i / 20) * 30 },
    }, p));
    if (threeD) {
      const group = PM.mkLayer('group', { threeD: true, p: { 'rotation.y': 15 } }, p);
      p.layers.forEach((layer: any) => layer.group = group.id);
      p.layers.unshift(group);
    }
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    PM.ProjectsScreen.hide(); PM.perf.auto = false; PM.quality = 1;
    (window as any).warmCalls = 0;
    const raster = PM.raster;
    PM.raster = (...args: any[]) => { if (args[2] === 4 && PM.time < 4) (window as any).warmCalls++; return raster(...args); };
    PM.setTime(paused ? 119 / 30 : 2.1);
    if (!paused) PM.play();
  }, {paused, threeD});
  await expect.poll(() => page.evaluate(() => (window as any).warmCalls), { timeout: 1800 }).toBeGreaterThanOrEqual(200);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const gl = PM.GL.gl, upload = gl.texImage2D.bind(gl);
    let uploads = 0;
    // Source uploads use the DOM-source overload. Nine-argument calls reserve
    // empty render-target storage, which is separate from source preparation.
    gl.texImage2D = (...args: any[]) => { if (args.length === 6) uploads++; return upload(...args); };
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

}

test('prepares zero-start text and shapes before their own or nested group opacity entrance', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Opacity entrance', w: 640, h: 360, dur: 10, fps: 30 });
    const outer = PM.mkLayer('group', {}, p), inner = PM.mkLayer('group', {}, p);
    inner.group = outer.id;
    const opacity = (layer: any) => layer.p.opacity.kf = [PM.KF(4, 0, 'linear'), PM.KF(4.5, 100, 'linear')];
    opacity(outer);
    p.layers = [outer, inner, ...Array.from({ length: 80 }, (_, i) => {
      const layer = PM.mkLayer(i % 2 ? 'shape' : 'text', {
        from: 0, dur: 10,
        d: i % 2 ? { w: 18, h: 18, color: '#' + (0x100000 + i * 151).toString(16) }
          : { text: String(i), size: 18, color: '#ffffff' },
        p: { 'position.x': 20 + i % 16 * 38, 'position.y': 30 + Math.floor(i / 16) * 60 },
      }, p);
      if (i < 40) layer.group = inner.id; else opacity(layer);
      return layer;
    })];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p }));
    PM.ProjectsScreen.hide(); PM.perf.auto = false; PM.quality = 1; PM.pause();
    (window as any).warmLayers = new Set();
    const raster = PM.raster;
    PM.raster = (...args: any[]) => {
      if (args[2] > 4 && args[2] < 4.5 && PM.time < 4) (window as any).warmLayers.add(args[0].id);
      return raster(...args);
    };
    PM.setTime(119 / 30);
  });
  await expect.poll(() => page.evaluate(() => (window as any).warmLayers.size), { timeout: 1800 }).toBe(80);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM, timeBefore = PM.time;
    PM.agentFrameCapture = true;
    const gl = PM.GL.gl, upload = gl.texImage2D.bind(gl);
    let sourceUploads = 0;
    gl.texImage2D = (...args: any[]) => { if (args.length === 6) sourceUploads++; return upload(...args); };
    PM.GL.render(121 / 30, { mblur: false });
    const first = new Uint8Array(gl.drawingBufferWidth * gl.drawingBufferHeight * 4);
    gl.readPixels(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, gl.RGBA, gl.UNSIGNED_BYTE, first);
    gl.texImage2D = upload;
    PM.rasterClear();
    PM.GL.render(121 / 30, { mblur: false, sourceClipping: false });
    const reference = new Uint8Array(first.length);
    gl.readPixels(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, gl.RGBA, gl.UNSIGNED_BYTE, reference);
    let maxDifference = 0;
    for (let i = 0; i < first.length; i++) maxDifference = Math.max(maxDifference, Math.abs(first[i]! - reference[i]!));
    return { sourceUploads, maxDifference, timeBefore, timeAfter: PM.time, draws: PM.GL.stats.draws };
  });
  expect(result.sourceUploads).toBe(0);
  expect(result.maxDifference).toBeLessThanOrEqual(1);
  expect(result.timeBefore).toBe(119 / 30);
  expect(result.timeAfter).toBe(result.timeBefore);
  expect(result.draws).toBeGreaterThanOrEqual(80);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

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

for (const depth of [null, 0, 200, -100]) {
test(`group effects preserve pixels with clipped sources at depth ${depth}`, async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  const result = await session.page.evaluate(depth => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Large blurred group', w: 320, h: 180, dur: 5 });
    const group = PM.mkLayer('group', { threeD: depth !== null, p: { 'position.z': depth ?? 0 } }, p);
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
  }, depth);
  expect(result.maxDifference).toBeLessThanOrEqual(1);
  expect(result.passes).toBeGreaterThan(0);
  expect(result.clippedBytes).toBeLessThan(result.referenceBytes / 4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

}
