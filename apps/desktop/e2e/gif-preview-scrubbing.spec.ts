import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test('GIF scrubbing and moving a clip under a fixed playhead show the requested source frame', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ w: 160, h: 90, fps: 30, dur: 2, name: 'GIF preview' }) }));
    PM.ProjectsScreen.hide(); PM.setTime(0, { force: true });
  });
  await importFixture(session.page, 'animated.gif');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((item: any) => item.type === 'video' && item.name === 'animated.gif'));
  const samples = await session.page.evaluate(async () => {
    const PM = (window as any).PM; PM.pause(); PM.perf.auto = false; PM.quality = 1;
    PM.GL.previewViewport = null; PM.GL.resize(160, 90);
    const layer = PM.proj.layers.find((item: any) => item.type === 'video');
    const samples: { frame: number; mode: string; pixel: number[] }[] = [];
    const capture = async (frame: number, mode: string) => {
      // A settled paused preview must agree with the GIF's known red/green/blue frames.
      const deadline = performance.now() + 3000;
      let pixel = new Uint8Array(4);
      do {
        await new Promise(resolve => requestAnimationFrame(resolve));
        const rendered = PM.GL.render(PM.time);
        if (rendered === false) continue;
        const gl = PM.GL.gl; gl.readPixels(40, 45, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
        if (pixel[frame]! > 200 && pixel[(frame + 1) % 3]! < 60) break;
      } while (performance.now() < deadline);
      samples.push({ frame, mode, pixel: [...pixel] });
    };
    for (const frame of [2, 0, 1, 2, 1, 0]) {
      PM.setTime(frame / 10, { raw: true, force: true }); await capture(frame, 'scrub');
    }
    PM.setTime(1, { raw: true, force: true });
    for (const frame of [2, 0, 1, 0, 2]) {
      layer.from = 1 - frame / 10; PM.touch(); PM.bus.emit('layers'); PM.invalidate();
      await capture(frame, 'move');
    }
    return samples;
  });
  for (const sample of samples) {
    expect(sample.pixel[sample.frame], JSON.stringify(sample)).toBeGreaterThan(200);
    expect(sample.pixel[(sample.frame + 1) % 3], JSON.stringify(sample)).toBeLessThan(60);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
