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

  test('preserves pixels when a non-zero-start layer is precomposed', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const result = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Precomp clock', w: 64, h: 64, fps: 30, dur: 4, bg: '#000000' });
      const solid = PM.mkLayer('solid', {
        name: 'Late solid', from: 2, dur: 1, d: { color: '#FFFFFF', w: 64, h: 64 },
      }, project);
      project.layers = [solid];
      PM.replaceProject(project);
      const brightness = () => {
        const canvas = PM.renderFrameTo(2.5, 64, 64) as HTMLCanvasElement;
        const data = canvas.getContext('2d')!.getImageData(0, 0, 64, 64).data;
        let sum = 0;
        for (let i = 0; i < data.length; i += 4) sum += data[i]! + data[i + 1]! + data[i + 2]!;
        return sum;
      };
      const before = brightness();
      const precomp = PM.precompose([solid.id], 'Late precomp');
      const after = brightness();
      const nested = PM.proj.comps[precomp.d.comp];
      return { before, after, outerFrom: precomp.from, innerFrom: nested.layers[0].from };
    });

    expect(result.before).toBeGreaterThan(0);
    expect(result.after).toBeGreaterThan(result.before * 0.99);
    expect(result.outerFrom).toBe(2);
    expect(result.innerFrom).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
