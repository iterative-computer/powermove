import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.describe('@import-mp4 H.264 import smoke', () => {
  test('imports H.264/AAC media and renders its green frame', async ({ session }) => {
    const { page } = session;
    await importFixture(page, 'h264-aac.mp4');

    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      return [...PM.assets.map.values()].some((asset: any) => asset.name === 'h264-aac.mp4')
        && PM.proj.layers.some((layer: any) => layer.type === 'video' && layer.name === 'h264-aac.mp4');
    });

    // Import places the clip at the playhead; pin it to t=0 through the typed
    // edit boundary so the fixture's 1.5s frame (green) lands at T=1.5. The
    // engine owns the <video> element's currentTime — never seek it directly.
    const applied = await page.evaluate(() => {
      const PM = (window as any).PM;
      return PM.Edit.apply(
        [{ type: 'set_layer', target: 'h264-aac.mp4', patch: { from: 0 } }],
        { label: 'e2e: pin clip to 0' }
      );
    });
    expect(applied.ok).toBe(true);

    // The compositor scrubs the element to the layer-local time and uploads the
    // frame once the decoder delivers it — poll the rendered pixel.
    const pixel = await page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.setTime(1.5, { raw: true, force: true });
      const sample = (): number[] => {
        const canvas = PM.renderFrameTo(1.5, PM.proj.w, PM.proj.h) as HTMLCanvasElement;
        const context = canvas.getContext('2d')!;
        return [...context.getImageData(Math.floor(canvas.width / 2), Math.floor(canvas.height / 2), 1, 1).data];
      };
      const isGreen = (p: number[]): boolean => p[0]! <= 30 && p[1]! >= 225 && p[2]! <= 30;
      const deadline = Date.now() + 10_000;
      let last = sample();
      while (Date.now() < deadline) {
        if (isGreen(last) && isGreen((last = sample()))) return last; // stable across two renders
        await new Promise(resolve => setTimeout(resolve, 120));
        last = sample();
      }
      return last;
    });

    expect(pixel[0]).toBeGreaterThanOrEqual(0);
    expect(pixel[0]).toBeLessThanOrEqual(30);
    expect(pixel[1]).toBeGreaterThanOrEqual(225);
    expect(pixel[1]).toBeLessThanOrEqual(255);
    expect(pixel[2]).toBeGreaterThanOrEqual(0);
    expect(pixel[2]).toBeLessThanOrEqual(30);
  });
});
