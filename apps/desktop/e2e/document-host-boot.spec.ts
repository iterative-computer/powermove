import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test, launchApp } from './helpers/app';

// Explicit measurement harness: real hidden app boots, alternating fresh
// profiles, one Electron instance at a time under pm-electron-lock.
test('new-engine UI installation does not wait for the document mirror or fonts', async () => {
  test.setTimeout(240_000);
  const samples: Array<{ engine: boolean; uiMs: number; editorMs: number; failures: unknown[] }> = [];
  for (let round = 0; round < 5; round++) for (const enabled of [false, true]) {
    const start = performance.now();
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
    try {
      await session.page.locator('#projects-screen').waitFor({ state: 'visible' });
      const uiMs = performance.now() - start;
      await session.openEditor();
      const editorMs = performance.now() - start;
      const failures = await session.page.evaluate(() => (window as any).PM.EngineDoc?.failures?.() ?? []);
      samples.push({ engine: enabled, uiMs, editorMs, failures });
      expect(failures).toEqual([]);
    } finally { await session.close(); }
  }
  const evidence = process.env.POWERMOVE_BOOT_EVIDENCE;
  if (evidence) { mkdirSync(evidence, { recursive: true }); writeFileSync(path.join(evidence, 'boot.json'), JSON.stringify(samples, null, 2)); }
  console.log('DOCUMENT_HOST_BOOT', JSON.stringify(samples));
});
