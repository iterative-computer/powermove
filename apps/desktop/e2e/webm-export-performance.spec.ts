import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

test('retains static WebM pictures while preserving timestamps and every encoded frame', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const output = path.join(session.userData, 'static.webm');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, output);
  const metrics = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Static WebM', w: 1280, h: 720, dur: 1, fps: 30, bg: '#21344a' });
    project.layers = Array.from({ length: 40 }, (_, i) => PM.mkLayer('shape', {
      d: { w: 170, h: 140, radius: 17, color: i % 2 ? '#a34bf7' : '#2bd773' },
      p: { 'position.x': 80 + i % 10 * 120, 'position.y': 80 + Math.floor(i / 10) * 160, opacity: 37, rotation: i * 7 },
    }, project));
    PM.proj = project; PM.touch();
    const render = PM.GL.render, results = [];
    for (let run = 0; run < 3; run++) {
      let renders = 0;
      PM.GL.render = (...args: any[]) => { renders++; return render(...args); };
      try {
        const start = performance.now();
        const result = await PM.Export.run({ format: 'webm', scale: 1, fps: 30, range: 'all', quality: 'draft', audio: false, mblur: false });
        results.push({ result, renders, ms: performance.now() - start });
      } finally { PM.GL.render = render; }
    }
    return results;
  });
  console.log('WEBM_EXPORT', JSON.stringify(metrics));
  for (const run of metrics) {
    expect(run.result).toEqual({ cancelled: false });
    if (process.env.PM_WEBM_EXPORT_BASELINE !== '1') expect(run.renders).toBe(1);
  }
  const decoded = execFileSync(ffmpegBinary,
    ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded.length).toBe(30 * 3);
  for (let i = 3; i < decoded.length; i++) expect(Math.abs(decoded[i]! - decoded[i % 3]!)).toBeLessThan(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('WebM reuse refreshes its picture at fractional layer entrances and exits', async ({ session }) => {
  await session.openEditor();
  const output = path.join(session.userData, 'cuts.webm');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, output);
  expect(await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Cuts', w: 64, h: 64, fps: 10, dur: 2, bg: '#21344a' });
    project.layers = [PM.mkLayer('solid', { from: .5 + .5e-6, dur: 1, d: { color: '#ff0000' } }, project)];
    PM.proj = project; PM.touch();
    return PM.Export.run({ format: 'webm', scale: 1, fps: 10, range: 'all', audio: false, mblur: false, quality: 'high' });
  })).toEqual({ cancelled: false });
  const decoded = execFileSync(ffmpegBinary,
    ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
  expect(decoded.length).toBe(20 * 3);
  for (let i = 0; i < 20; i++) {
    const expected = i >= 5 && i < 15 ? [255, 0, 0] : [0x21, 0x34, 0x4a];
    for (const [channel, value] of expected.entries()) expect(Math.abs(decoded[i * 3 + channel]! - value)).toBeLessThan(5);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
