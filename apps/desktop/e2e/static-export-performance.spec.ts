import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

test('exports a solid background without recapturing every frame', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  await session.app.evaluate(({ dialog }, root) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: `${root}/solid.mp4` });
  }, session.userData);
  const metrics = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    PM.proj = PM.mkProject({ name: 'Solid export', w: 1920, h: 1080, dur: 3, fps: 30, bg: '#21344a' }); PM.touch();
    const original = PM.GL.renderToPixels;
    let captures = 0;
    PM.GL.renderToPixels = (...args: any[]) => { captures++; return original(...args); };
    try {
      const start = performance.now();
      const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'draft', alpha: false, audio: false, mblur: false });
      return { captures, ms: performance.now() - start, result };
    } finally { PM.GL.renderToPixels = original; }
  });
  console.log('STATIC_EXPORT', JSON.stringify(metrics));
  expect(metrics.result).toEqual({ cancelled: false });
  expect(metrics.captures).toBe(1);
  const decoded = execFileSync(ffmpegBinary,
    ['-v', 'error', '-i', path.join(session.userData, 'solid.mp4'), '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded.length).toBe(90 * 3);
  for (let i = 0; i < decoded.length; i += 3) {
    for (const [channel, value] of [0x21, 0x34, 0x4a].entries()) expect(Math.abs(decoded[i + channel]! - value)).toBeLessThan(5);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const alpha of [false, true]) test(`retains static frames across layer entrances and exits (${alpha ? 'alpha' : 'opaque'})`, async ({ session }) => {
  await session.openEditor();
  const output = path.join(session.userData, alpha ? 'cuts.mov' : 'cuts.mp4');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, output);
  const result = await session.page.evaluate(async alpha => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Static cuts', w: 64, h: 64, fps: 10, dur: 2, bg: '#21344a' });
    if (alpha) project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    project.layers = [PM.mkLayer('solid', { from: .5, dur: 1, d: { color: '#ff0000' } }, project)];
    PM.proj = project; PM.touch();
    let captures = 0;
    const original = PM.GL.renderToPixels;
    PM.GL.renderToPixels = (...args: any[]) => { captures++; return original(...args); };
    try {
      const result = await PM.Export.run({ format: alpha ? 'prores' : 'mp4', alpha, scale: 1, fps: 10, range: 'all', quality: 'draft', audio: false, mblur: false });
      return { result, captures };
    } finally { PM.GL.renderToPixels = original; }
  }, alpha);
  expect(result).toEqual({ result: { cancelled: false }, captures: 3 });
  const decoded = execFileSync(ffmpegBinary,
    ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgba', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded.length).toBe(20 * 4);
  for (let i = 0; i < 20; i++) {
    const active = i >= 5 && i < 15;
    const expected = active ? [255, 0, 0, 255] : alpha ? [0, 0, 0, 0] : [0x21, 0x34, 0x4a, 255];
    for (const [channel, value] of expected.entries()) expect(Math.abs(decoded[i * 4 + channel]! - value)).toBeLessThan(5);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
