import { expect, test } from './helpers/app';

test.describe('@viewer adjustment layers', () => {
  test('processes only lower layers across the full frame with timing, masks, and opacity', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

    const proof = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Adjustment proof', w: 16, h: 16, fps: 30, dur: 4, bg: '#000000' });
      const bottom = PM.mkLayer('solid', {
        name: 'Red below', from: 0, dur: 4, d: { color: '#FF0000', w: 16, h: 16 },
      }, project);
      const adjustment = PM.mkLayer('adjustment', { name: 'Invert below', from: 1, dur: 1 }, project);
      adjustment.fx.push(PM.mkEffect('invert'));
      const top = PM.mkLayer('solid', {
        name: 'Green above', from: 0, dur: 4, d: { color: '#00FF00', w: 4, h: 4 },
      }, project);
      top.p['position.x'].v = 6;
      top.p['position.y'].v = 6;
      project.layers = [top, adjustment, bottom];
      PM.replaceProject(project);

      const pixel = (time: number, x: number, y: number) => {
        const canvas = PM.renderFrameTo(time, 16, 16) as HTMLCanvasElement;
        return Array.from(canvas.getContext('2d')!.getImageData(x, y, 1, 1).data);
      };
      const timedOut = pixel(.5, 1, 1);
      const adjusted = pixel(1.5, 1, 1);
      const above = pixel(1.5, 7, 7);
      const ended = pixel(2.5, 1, 1);

      adjustment.p.opacity.v = 50;
      PM.touch();
      const half = pixel(1.5, 1, 1);

      adjustment.p.opacity.v = 100;
      const mask = PM.mkMask('rect', project);
      mask.p.x.v = 8;
      mask.p.y.v = 8;
      mask.p.w.v = 8;
      mask.p.h.v = 8;
      mask.p.feather.v = 0;
      adjustment.masks = [mask];
      PM.touch();
      const maskedOutside = pixel(1.5, 1, 1);
      const maskedInside = pixel(1.5, 10, 10);

      return { timedOut, adjusted, above, ended, half, maskedOutside, maskedInside };
    });

    expect(proof.timedOut.slice(0, 3)).toEqual([255, 0, 0]);
    expect(proof.adjusted.slice(0, 3)).toEqual([0, 255, 255]);
    expect(proof.above.slice(0, 3)).toEqual([0, 255, 0]);
    expect(proof.ended.slice(0, 3)).toEqual([255, 0, 0]);
    for (const channel of proof.half.slice(0, 3)) expect(channel).toBeGreaterThanOrEqual(126);
    for (const channel of proof.half.slice(0, 3)) expect(channel).toBeLessThanOrEqual(129);
    expect(proof.maskedOutside.slice(0, 3)).toEqual([255, 0, 0]);
    expect(proof.maskedInside.slice(0, 3)).toEqual([0, 255, 255]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
