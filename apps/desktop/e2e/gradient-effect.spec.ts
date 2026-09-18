import { expect, test } from './helpers/app';

test.describe('@viewer gradient ramp effect', () => {
  test('ramps linearly, radially, and only inside the layer alpha', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

    const proof = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Gradient proof', w: 16, h: 16, fps: 30, dur: 4, bg: '#000000' });
      const fill = PM.mkLayer('solid', {
        name: 'White fill', from: 0, dur: 4, d: { color: '#FFFFFF', w: 16, h: 16 },
      }, project);
      const effect = PM.mkEffect('gradient');
      fill.fx.push(effect);
      project.layers = [fill];
      PM.replaceProject(project);

      const pixel = (x: number, y: number) => {
        const canvas = PM.renderFrameTo(1, 16, 16) as HTMLCanvasElement;
        return Array.from(canvas.getContext('2d')!.getImageData(x, y, 1, 1).data) as number[];
      };
      const set = (values: Record<string, unknown>) => {
        for (const [key, value] of Object.entries(values)) effect.p[key].v = value;
        PM.touch();
      };

      /* Defaults: white → black travelling top to bottom at 90°. */
      const top = pixel(8, 1), middle = pixel(8, 8), bottom = pixel(8, 14);

      set({ angle: 0, startColor: '#FF0000', endColor: '#0000FF' });
      const left = pixel(1, 8), right = pixel(14, 8);

      set({ angle: 90, startColor: '#FFFFFF', endColor: '#000000', radial: true });
      const center = pixel(8, 8), corner = pixel(1, 1);

      set({ radial: false, amount: 0 });
      const off = pixel(8, 14);

      /* A smaller solid proves the ramp respects the layer's own alpha. */
      const small = PM.mkProject({ name: 'Gradient alpha proof', w: 16, h: 16, fps: 30, dur: 4, bg: '#000000' });
      const patch = PM.mkLayer('solid', {
        name: 'Small fill', from: 0, dur: 4, d: { color: '#FFFFFF', w: 4, h: 4 },
      }, small);
      patch.p['position.x'].v = 6;
      patch.p['position.y'].v = 6;
      patch.fx.push(PM.mkEffect('gradient'));
      small.layers = [patch];
      PM.replaceProject(small);
      const outside = pixel(1, 1), inside = pixel(7, 7);

      return { top, middle, bottom, left, right, center, corner, off, outside, inside };
    });

    const value = (rgba: number[]) => rgba[0]!;
    /* Linear ramp: bright at the start edge, dark at the end edge, monotonic. */
    expect(value(proof.top)).toBeGreaterThan(value(proof.middle));
    expect(value(proof.middle)).toBeGreaterThan(value(proof.bottom));
    expect(value(proof.top)).toBeGreaterThan(220);
    expect(value(proof.bottom)).toBeLessThan(60);

    /* Angle 0 runs the ramp left to right between the two colours. */
    expect(proof.left[0]).toBeGreaterThan(200);
    expect(proof.left[2]).toBeLessThan(60);
    expect(proof.right[2]).toBeGreaterThan(200);
    expect(proof.right[0]).toBeLessThan(60);

    /* Radial: the start colour sits at the centre, the end colour at the rim. */
    expect(value(proof.center)).toBeGreaterThan(220);
    expect(value(proof.corner)).toBeLessThan(60);

    /* Amount 0 leaves the source untouched. */
    expect(proof.off.slice(0, 3)).toEqual([255, 255, 255]);

    /* Transparent pixels keep the background; the ramp paints the solid only. */
    expect(proof.outside.slice(0, 3)).toEqual([0, 0, 0]);
    expect(value(proof.inside)).toBeGreaterThan(60);

    expect(session.diagnostics.pageErrors).toEqual([]);
    expect(session.diagnostics.console.filter((entry) => entry.type === 'error')).toEqual([]);
  });
});
