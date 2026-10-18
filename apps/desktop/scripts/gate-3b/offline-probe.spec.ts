import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp } from '../../e2e/helpers/app';
import { importFixture } from '../../e2e/helpers/media';

/* Triage probe for the Phase 3b gate (docs/engine/phase3b-gate.md): with
   Use New Engine on, `export-mp4.spec.ts` and other legacy export specs get
   `{cancelled: true}` from `PM.Export.run`. This replays export-mp4's steps
   (a project installed by assigning `PM.proj`, then a WAV imported through
   the file picker) and records every native message main shows and what the
   media store resolves for each asset, answering each message with its
   Cancel button as a hidden test window does. Diagnostic only: runs with
   PM_GATE_3B=1. */

test('probe: what cancels a legacy export spec with Use New Engine on', async () => {
  test.skip(process.env['PM_GATE_3B'] !== '1', 'diagnostic: set PM_GATE_3B=1');
  test.setTimeout(120_000);
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    const { app, page } = session;
    await session.openEditor();
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.assets.clear();
      PM.proj = PM.mkProject({ name: 'Probe', w: 64, h: 64, fps: 30, dur: 1, bg: '#000000' });
      PM.proj.work = [0, 1];
      PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Red', from: 0, duration: 1, content: { color: '#FF0000', w: 64, h: 64 } }], { label: 'probe', origin: 'e2e' });
      PM.bus.emit('project');
      PM.bus.emit('layers');
    });
    await importFixture(page, 'tone.wav');
    await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'audio'));
    const output = path.join(session.userData, 'Probe.mp4');
    await app.evaluate(({ dialog }, file) => {
      const g = globalThis as any;
      g.__messages = [];
      dialog.showSaveDialog = (async () => ({ canceled: false, filePath: file })) as any;
      dialog.showMessageBox = (async (...args: any[]) => {
        const options = args.length > 1 ? args[1] : args[0];
        g.__messages.push({ message: options.message, detail: options.detail, buttons: options.buttons });
        return { response: options.cancelId ?? 0, checkboxChecked: false };
      }) as any;
    }, output);
    const assets = await page.evaluate(() => Object.keys((window as any).PM.proj.assets ?? {}));
    const result = await page.evaluate(async () => (window as any).PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'draft', mblur: false, alpha: false, audio: true }));
    const messages = await app.evaluate(() => (globalThis as any).__messages);
    console.log(`PROBE assets ${JSON.stringify(assets)}`);
    console.log(`PROBE export result ${JSON.stringify(result)}`);
    console.log(`PROBE native messages ${JSON.stringify(messages)}`);
    // A second try after a second: does the binding arrive late?
    await page.waitForTimeout(1500);
    await app.evaluate(() => { (globalThis as any).__messages = []; });
    const again = await page.evaluate(async () => (window as any).PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'draft', mblur: false, alpha: false, audio: true }));
    console.log(`PROBE export after 1.5 s ${JSON.stringify(again)}; messages ${JSON.stringify(await app.evaluate(() => (globalThis as any).__messages))}`);
    expect(true).toBe(true);
  } finally {
    await session.close();
  }
});
