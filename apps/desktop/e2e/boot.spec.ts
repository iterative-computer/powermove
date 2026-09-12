import { expect, test } from './helpers/app';

// Intentionally narrow: Chromium occasionally reports this while a page is
// closing, after the assertions have completed. Any new renderer error fails.
const CONSOLE_ERROR_ALLOWLIST = [/^Request Autofill\.enable failed/];

test.describe('@boot Electron boot smoke', () => {
  test('opens the legacy renderer with WebGL and the demo project', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await page.waitForTimeout(1_300); // include deferred native boot diagnostics/errors

    const state = await page.evaluate(() => {
      const PM = (window as any).PM;
      return {
        hasWebGL: Boolean(PM?.GL?.gl),
        layers: PM?.proj?.layers?.length ?? 0
      };
    });

    expect(state.hasWebGL).toBe(true);
    expect(state.layers).toBeGreaterThan(0);
    expect(diagnostics.pageErrors).toEqual([]);
    const unexpectedErrors = diagnostics.console.filter(record =>
      record.type === 'error' && !CONSOLE_ERROR_ALLOWLIST.some(pattern => pattern.test(record.text))
    );
    expect(unexpectedErrors).toEqual([]);
  });

  test('exposes a usable WebGPU adapter when the capability exists', async ({ session }) => {
    const capability = await session.page.evaluate(async () => {
      const gpu = (navigator as any).gpu;
      if (!gpu) return { present: false, adapter: false };
      return { present: true, adapter: Boolean(await gpu.requestAdapter()) };
    });

    test.skip(!capability.present, 'WebGPU is not exposed by this Electron/host combination');
    expect(capability.adapter).toBe(true);
  });
});
