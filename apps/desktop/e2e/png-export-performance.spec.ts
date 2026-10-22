import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { expect, test, repoRoot } from './helpers/app';

test('reuses and releases one opaque PNG surface while preserving every animated frame', async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ dialog }, folder) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
  }, session.userData);
  const metrics = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Animated surface', w: 96, h: 64, fps: 6, dur: 1, bg: '#21344a' });
    const shape = PM.mkLayer('shape', { d: { w: 39, h: 43, radius: 11, color: '#dd7355' },
      p: { 'position.x': 16, 'position.y': 30, opacity: 63, rotation: 17 } }, project);
    project.layers = [shape]; PM.proj = project; PM.touch();
    PM.animate(shape, 'position.x', [{ t: 0, v: 16 }, { t: 1, v: 80 }]);
    const expected: number[] = [];
    for (let frame = 0; frame < 6; frame++) {
      const canvas = PM.renderFrameTo(frame / 6, 96, 64, { mblur: false });
      expected.push(...canvas.getContext('2d').getImageData(0, 0, 96, 64).data);
      canvas.width = 0; canvas.height = 0;
    }
    const capture = PM.GL.renderToPixels, encode = HTMLCanvasElement.prototype.toBlob;
    const surfaces = new Set<HTMLCanvasElement>();
    let captures = 0, encodes = 0;
    PM.GL.renderToPixels = (...args: any[]) => { captures++; return capture(...args); };
    HTMLCanvasElement.prototype.toBlob = function (...args: any[]) {
      encodes++; surfaces.add(this); return (encode as any).apply(this, args);
    };
    try {
      const result = await PM.Export.run({ format: 'png', alpha: false, scale: 1, fps: 6, range: 'all', audio: false, mblur: false });
      return { result, expected, captures, encodes, surfaces: surfaces.size,
        released: [...surfaces].every(canvas => canvas.width === 0 && canvas.height === 0) };
    } finally { PM.GL.renderToPixels = capture; HTMLCanvasElement.prototype.toBlob = encode; }
  });
  expect(metrics.result).toEqual({ cancelled: false });
  expect(metrics.captures).toBe(6);
  expect(metrics.encodes).toBe(6);
  expect(metrics.surfaces).toBe(1);
  expect(metrics.released).toBe(true);
  const decoded = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
    ['-v', 'error', '-i', path.join(session.userData, 'Animated surface_%05d.png'), '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded).toEqual(Buffer.from(metrics.expected));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const { alpha, depth } of [{ alpha: false, depth: 16 }, { alpha: true, depth: 16 }, { alpha: true, depth: 32 }]) test(`reuses identical PNG frames (${alpha ? 'alpha' : 'opaque'}, ${depth}-bit)`, async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ dialog }, folder) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
  }, session.userData);
  const metrics = await session.page.evaluate(async ({ alpha, depth }) => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Static PNG', w: 1920, h: 1080, dur: 1, fps: 24, bg: '#21344a' });
    project.colorDepth = depth;
    if (alpha) project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    project.layers = [PM.mkLayer('shape', { d: { w: 990, h: 670, radius: 17, color: '#a34bf7' }, p: { 'position.x': 900, 'position.y': 490, opacity: 37, rotation: 17 } }, project)];
    PM.proj = project; PM.touch();
    const capture = PM.GL.renderToPixels, encode = HTMLCanvasElement.prototype.toBlob;
    const factories = ['createHalfReadback', 'createFloatReadback'].map(name => [name, PM.GL[name]] as const);
    let captures = 0, encodes = 0;
    PM.GL.renderToPixels = (...args: any[]) => { captures++; return capture(...args); };
    for (const [name, factory] of factories) if (factory) PM.GL[name] = (...args: any[]) => {
      const session = factory(...args);
      if (!session) return session;
      return {
        capture(...args: any[]) {
          const before = captures;
          const pixels = session.capture(...args);
          // The private compositor capture also owns its original fallback.
          // Count one rendered picture whichever provider produced its bytes.
          if (captures === before) captures++;
          return pixels;
        },
        dispose() { session.dispose(); },
      };
    };
    HTMLCanvasElement.prototype.toBlob = function (...args: any[]) { encodes++; return (encode as any).apply(this, args); };
    try {
      const start = performance.now();
      const result = await PM.Export.run({ format: 'png', alpha, scale: 1, fps: 24, range: 'all', audio: false, mblur: false });
      return { result, captures, encodes, ms: performance.now() - start };
    } finally {
      PM.GL.renderToPixels = capture;
      for (const [name, factory] of factories) PM.GL[name] = factory;
      HTMLCanvasElement.prototype.toBlob = encode;
    }
  }, { alpha, depth });
  console.log('PNG_EXPORT', JSON.stringify({ alpha, depth, ...metrics }));
  expect(metrics.result).toEqual({ cancelled: false });
  if (process.env.PM_PNG_EXPORT_BASELINE !== '1') {
    expect(metrics.captures).toBe(1);
    expect(metrics.encodes).toBe(1);
  }
  const first = await readFile(path.join(session.userData, 'Static PNG_00000.png'));
  for (let i = 1; i < 24; i++) expect(await readFile(path.join(session.userData, `Static PNG_${String(i).padStart(5, '0')}.png`))).toEqual(first);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const alpha of [false, true]) test(`PNG reuse preserves fractional layer entrances and exits (${alpha ? 'alpha' : 'opaque'})`, async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ dialog }, folder) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
  }, session.userData);
  expect(await session.page.evaluate(async alpha => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Cuts', w: 64, h: 64, fps: 10, dur: 2, bg: '#21344a' });
    if (alpha) project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    project.layers = [PM.mkLayer('solid', { from: .5 + .5e-6, dur: 1, d: { color: '#ff0000' } }, project)];
    PM.proj = project; PM.touch();
    return PM.Export.run({ format: 'png', alpha, scale: 1, fps: 10, range: 'all', audio: false, mblur: false });
  }, alpha)).toEqual({ cancelled: false });
  const decoded = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
    ['-v', 'error', '-i', path.join(session.userData, 'Cuts_%05d.png'), '-vf', 'scale=1:1', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded.length).toBe(20 * 4);
  for (let i = 0; i < 20; i++) expect([...decoded.subarray(i * 4, i * 4 + 4)]).toEqual(
    i >= 5 && i < 15 ? [255, 0, 0, 255] : alpha ? [0, 0, 0, 0] : [0x21, 0x34, 0x4a, 255]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('PNG reuse preserves grouped motion blur with a negative shutter', async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ dialog }, folder) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [folder] });
  }, session.userData);
  const reference = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Reversed shutter', w: 64, h: 64, fps: 30, dur: 2, bg: '#000000' });
    project.shutter = -.5; project.work = [.98, 1.02];
    const group = PM.mkLayer('group', {}, project);
    const child = PM.mkLayer('solid', { from: 1, dur: 1, d: { color: '#ff0000' } }, project);
    group.mblur = true; child.group = group.id;
    project.layers = [group, child]; PM.proj = project; PM.touch();
    const frames = Math.round((project.work[1] - project.work[0]) * 240);
    const pixels = Array.from({ length: frames }, (_, index) => {
      const canvas = PM.renderFrameTo(project.work[0] + index / 240, 64, 64, { mblur: true });
      return [...canvas.getContext('2d').getImageData(32, 32, 1, 1).data];
    });
    const result = await PM.Export.run({ format: 'png', alpha: false, scale: 1, fps: 240, range: 'work', audio: false, mblur: true });
    return { result, pixels };
  });
  expect(reference.result).toEqual({ cancelled: false });
  expect(new Set(reference.pixels.map(pixel => pixel[0])).size).toBeGreaterThan(2);
  const decoded = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
    ['-v', 'error', '-i', path.join(session.userData, 'Reversed shutter_%05d.png'), '-vf', 'crop=1:1:32:32', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded.length).toBe(reference.pixels.length * 4);
  for (const [index, pixel] of reference.pixels.entries()) expect([...decoded.subarray(index * 4, index * 4 + 4)]).toEqual(pixel);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
