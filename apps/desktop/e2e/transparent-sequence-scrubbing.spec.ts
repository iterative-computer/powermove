import { expect, test } from './helpers/app';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

for (const auto of [false, true]) test(`transparent sequence scrubbing replaces cached pixels (${auto ? 'Auto preview' : 'original'})`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const images = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ w: 128, h: 128, fps: 30, dur: 2 }) }));
    PM.ProjectsScreen.hide(); PM.setTime(0, { force: true });
    const files = [];
    for (let frame = 0; frame < 4; frame++) {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 2048;
      const ctx = canvas.getContext('2d')!;
      ctx.fillStyle = 'white'; ctx.fillRect(frame * 512, 1024, 512, 1024);
      ctx.fillStyle = 'rgba(255,255,255,0.5)'; ctx.fillRect(0, 0, 2048, 512);
      files.push(canvas.toDataURL('image/png').split(',')[1]!);
    }
    return files;
  });
  const paths = images.map((_, frame) => path.join(session.userData, `scrub_${frame}.png`));
  for (let frame = 0; frame < paths.length; frame++) await writeFile(paths[frame]!, Buffer.from(images[frame]!, 'base64'));
  const chooser = session.page.waitForEvent('filechooser');
  await session.page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(paths);
  const dialog = session.page.getByRole('dialog', { name: 'Import image sequence', exact: true });
  await dialog.getByRole('spinbutton', { name: 'Sequence frame rate' }).fill('10');
  await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'video'));
  const samples = await session.page.evaluate(async auto => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((layer: any) => layer.type === 'video');
    const asset = PM.assets.get(layer.d.asset);
    await asset.previewReady;
    if (auto && !asset.preview) throw new Error('Expected a preview proxy');
    PM.pause(); PM.perf.auto = auto; PM.quality = 1;
    layer.d.w = layer.d.h = 128;
    PM.GL.previewViewport = null; PM.GL.resize(128, 128);
    const samples = [];
    // Visit enough distinct frames to recycle both the one-frame original
    // cache and the two-frame proxy cache, then reverse direction.
    for (const frame of [1, 2, 3, 0, 3, 2, 1, 0]) {
      PM.setTime(frame / 10, { raw: true, force: true });
      const deadline = performance.now() + 3000;
      let ready = false;
      do {
        await new Promise(resolve => requestAnimationFrame(resolve));
        ready = PM.GL.render(PM.time, { mblur: false }) !== false;
      } while (!ready && performance.now() < deadline);
      if (!ready) throw new Error(`Frame ${frame} did not settle`);
      const gl = PM.GL.gl;
      const pixel = (x: number, y: number) => {
        const rgba = new Uint8Array(4); gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
        return rgba[0]!;
      };
      samples.push({ frame, bars: [16, 48, 80, 112].map(x => pixel(x, 32)), translucent: pixel(64, 112) });
    }
    return samples;
  }, auto);
  for (const sample of samples) {
    for (let bar = 0; bar < 4; bar++) {
      if (bar === sample.frame) expect(sample.bars[bar], JSON.stringify(sample)).toBeGreaterThan(240);
      else expect(sample.bars[bar], JSON.stringify(sample)).toBeLessThan(15);
    }
    expect(sample.translucent, JSON.stringify(sample)).toBeGreaterThanOrEqual(125);
    expect(sample.translucent, JSON.stringify(sample)).toBeLessThanOrEqual(131);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
