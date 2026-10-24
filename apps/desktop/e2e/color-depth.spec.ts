import { expect, test } from './helpers/app';

test('color depth settings select real GPU precision, preserve alpha, and undo', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  const supported = await page.evaluate(() => (window as any).PM.GL.supportedColorDepths);
  // This suite runs on a GPU with float rendering, blending and filtering.
  expect(supported).toEqual([8, 16, 32]);

  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Color depth proof', w: 16, h: 16, dur: 2 }));
    PM.Kernel.registerEffect('color-depth-proof', {
      id: 'precision-proof', label: 'Precision proof', group: 'Test', params: [],
      frag: 'vec4 c = src(v_st); o = vec4((c.rgb - vec3(.5)) * 1000., c.a);',
    });
    const layer = PM.mkLayer('shader', { name: 'Precision', dur: 2, d: {
      w: 16, h: 16, code: 'void main(){ fragColor = vec4(vec3(.5001), 1.); }',
    } });
    layer.fx.push(PM.mkEffect('precision-proof'));
    PM.proj.layers = [layer]; PM.touch();
  });

  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  const depth = settings.locator('select[aria-label="Color depth"]');
  await expect(depth).toHaveValue('16');

  const samples: Record<string, number[]> = {};
  for (const bits of [8, 16, 32, 16, 32]) {
    await depth.selectOption(String(bits));
    samples[bits] = await page.evaluate(() => {
      const PM = (window as any).PM;
      const pixels = PM.GL.renderToPixels(0, 16, 16, { exporting: true, mblur: false });
      if (PM.GL.gl.getError()) throw new Error('GPU error during color depth render');
      return Array.from(pixels.slice((8 * 16 + 8) * 4, (8 * 16 + 8) * 4 + 4)) as number[];
    });
    expect(await page.evaluate(() => {
      const PM = (window as any).PM;
      return { saved: PM.proj.colorDepth, actual: PM.GL.colorDepth, pool: PM.GL.pool.map((f: any) => f.colorDepth) };
    })).toMatchObject({ saved: bits, actual: bits, pool: expect.arrayContaining([bits]) });
  }
  // Recover a tiny difference after it passes through the working texture.
  expect(samples[8]![0]).toBe(255);
  expect(samples[16]![0]).toBe(0);
  expect(samples[32]![0]).toBeGreaterThanOrEqual(24);
  expect(samples[32]![0]).toBeLessThanOrEqual(27);

  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
    await settings.screenshot({ path: info.outputPath(`color-depth-settings-${theme}.png`) });
  }
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect(depth).toHaveValue('16');
  await page.evaluate(() => (window as any).PM.hist.redo());
  await expect(depth).toHaveValue('32');
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.hydrateProject(PM.deserialize(PM.serialize())).colorDepth;
  })).toBe(32);

  const alpha = await page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.mkLayer('solid', { name: 'Alpha', dur: 2, d: { color: '#FF000080', w: 16, h: 16 } });
    // Force the effect/composite path as well as the normal fast path.
    const result: any[] = [];
    PM.proj.layers = [layer];
    for (const bits of [8, 16, 32]) {
      PM.proj.colorDepth = bits;
      for (const effect of [false, true]) {
        layer.fx = effect ? [PM.mkEffect('invert')] : [];
        PM.touch();
        const pixels = PM.GL.renderToPixels(0, 16, 16, { exporting: true, mblur: false, transparent: true });
        result.push({ bits, effect, pixel: Array.from(pixels.slice(0, 4)), error: PM.GL.gl.getError() });
      }
    }
    return result;
  });
  for (const sample of alpha) {
    expect(sample.error).toBe(0);
    const expected = sample.effect ? [0, 128, 128, 128] : [128, 0, 0, 128];
    sample.pixel.forEach((value: number, channel: number) => expect(Math.abs(value - expected[channel]!)).toBeLessThanOrEqual(1));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
