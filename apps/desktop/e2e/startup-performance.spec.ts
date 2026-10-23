import { writeFileSync } from 'node:fs';
import { expect, launchApp, test } from './helpers/app';

// Run on the same machine/build without concurrent tests or builds. The
// fixture uses only disposable profiles; it never reads a live user's store.
test.skip(!process.env.PM_STARTUP_BENCH, 'Set PM_STARTUP_BENCH=1 for isolated startup measurements');
test('measures complete startup and retained heap on fresh profiles', async ({}, info) => {
  test.setTimeout(120_000);
  const samples: { readyMs: number; rendererHeapBytes: number; mainHeapBytes: number }[] = [];
  for (let pass = 0; pass < 5; pass++) {
    const started = performance.now();
    const session = await launchApp();
    try {
      await session.page.evaluate(async () => { await (window as any).PM.Kernel.loader.builtinsReady; });
      const readyMs = performance.now() - started;
      const cdp = await session.page.context().newCDPSession(session.page);
      await cdp.send('HeapProfiler.collectGarbage');
      const renderer = await cdp.send('Runtime.getHeapUsage');
      const mainHeapBytes = await session.app.evaluate(() => process.memoryUsage().heapUsed);
      samples.push({ readyMs, rendererHeapBytes: renderer.usedSize, mainHeapBytes });
      expect(session.diagnostics.pageErrors).toEqual([]);
      expect(await session.page.evaluate(() => (window as any).PM.ProjectsScreen.isOpen)).toBe(true);
      await cdp.detach();
    } finally { await session.close(); }
  }
  const result = { samples, medianReadyMs: [...samples].sort((a, b) => a.readyMs - b.readyMs)[2]!.readyMs };
  writeFileSync(info.outputPath('startup-metrics.json'), JSON.stringify(result, null, 2));
  console.log('STARTUP', JSON.stringify(result));
});
