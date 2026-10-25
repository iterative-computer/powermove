import path from 'node:path';
import { expect, launchApp, test } from './helpers/app';

// Exposure must recover highlights before display clamps them. Neutral
// chroma makes the independent reference exactly (Y - 16) / 219.
test('native video keeps superwhites available to exposure', async () => {
  test.setTimeout(90_000);
  for (const engine of [false, true]) {
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0' } });
    try {
      await session.openEditor();
      const errors: number[] = [];
      for (const [name, w, h] of [['sd', 256, 144], ['hd', 1280, 720]] as const) {
        const source = path.resolve(__dirname, `../../../engine/crates/pm-render/tests/fixtures/video/superwhite-${name}.mp4`);
        await session.page.evaluate(({ w, h }) => {
          const PM = (window as any).PM;
          const project = PM.mkProject({ name: 'Highlight recovery', w, h, fps: 30, dur: 1 });
          project.colorDepth = 32; PM.replaceProject(project);
          document.querySelector('#highlight-source')?.remove();
          const input = document.createElement('input'); input.type = 'file'; input.id = 'highlight-source'; document.body.append(input);
        }, { w, h });
        await session.page.locator('#highlight-source').setInputFiles(source);
        await session.page.evaluate(async () => {
          const PM = (window as any).PM;
          await PM.importFiles([(document.querySelector('#highlight-source') as HTMLInputElement).files![0]]);
          const video = PM.proj.layers.find((layer: any) => layer.type === 'video');
          video.from = 0; video.dur = 1; PM.pause(); PM.touch();
        });
        for (const exposure of [0, -1]) {
          const levels = await session.page.evaluate(async ({ engine, w, h, exposure }) => {
            const PM = (window as any).PM;
            const video = PM.proj.layers.find((layer: any) => layer.type === 'video');
            video.fx = exposure ? [{ id: 'exposure', type: 'color', on: true, p: { exposure: PM.P(exposure) } }] : [];
            PM.touch(); PM.setTime(0, { raw: true, force: true });
            let pixels: ArrayLike<number>;
            if (engine) {
              const snapshot = await PM.EngineRender.thumbnails.request({ key: 'highlight-recovery',
                composition: String(PM.proj.compId ?? PM.proj.id), time: 0, width: w, height: h, pixels: 'opaque' });
              pixels = PM.EngineRender.toCanvas(snapshot).getContext('2d').getImageData(0, 0, w, h).data;
            } else {
              PM.agentFrameCapture = true;
              try { await PM.prepareFrame(0); pixels = PM.GL.renderToPixels(0, w, h, { mblur: false, opaque: true, topDownOpaque: true }); }
              finally { PM.agentFrameCapture = false; }
            }
            return Array.from({ length: 5 }, (_, band) => pixels[(Math.floor((band * h + h / 2) / 5) * w + w / 2) * 4]!);
          }, { engine, w, h, exposure });
          for (const [band, code] of [8, 16, 128, 235, 250].entries()) {
            const expected = Math.max(0, Math.min(1, 2 ** exposure * (code - 16) / 219)) * 255;
            const error = Math.abs(levels[band]! - expected);
            errors.push(error);
            if (engine) expect(error, `${name} ${exposure} stops band ${code}: ${levels[band]} vs ${expected}`).toBeLessThan(1);
          }
        }
      }
      errors.sort((a, b) => a - b);
      console.log('SUPERWHITE-DISPLAY', JSON.stringify({ engine, max: errors.at(-1), p999: errors.at(-1),
        mean: errors.reduce((a, b) => a + b, 0) / errors.length, samples: errors.length }));
    } finally { await session.close(); }
  }
});
