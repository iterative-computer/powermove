import { expect, test } from './helpers/app';

test.describe('@precomp nested composition rendering', () => {
  test('precomposing layers renders without throwing and produces pixels', async ({ session }) => {
    const { page } = session;
    // The viewer is a built-in extension now, so its WebGL host activates
    // asynchronously after the legacy registry first becomes observable.
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const result = await page.evaluate(() => {
      const PM = (window as any).PM;
      const ids = PM.proj.layers.slice(0, 2).map((layer: any) => layer.id);
      const precomp = PM.hist.do('Precompose', () => PM.precompose(ids, 'E2E precomp'));
      PM.setTime(0.5, { raw: true, force: true });
      const canvas = PM.renderFrameTo(0.5, PM.proj.w, PM.proj.h) as HTMLCanvasElement;
      const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
      let lit = 0;
      for (let i = 0; i < data.length; i += 4 * 97) if (data[i]! + data[i + 1]! + data[i + 2]! > 30) lit++;
      return { precompType: precomp?.type, comps: Object.keys(PM.proj.comps).length, lit };
    });
    expect(result.precompType).toBe('precomp');
    expect(result.comps).toBe(1);
    // The legacy compositor threw a ReferenceError on this path; a thrown frame
    // leaves the accumulation buffer black.
    expect(result.lit).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
