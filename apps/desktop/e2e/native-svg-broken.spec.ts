/* Broken persisted SVG assets have the same empty paint as today's <img>.
   This does not establish parity for valid SVG paint records. */
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect, launchApp } from './helpers/app';

const sources = [
  ['truncated', '<svg xmlns="http://www.w3.org/2000/svg"><rect'],
  ['wrong-root', '<html xmlns="http://www.w3.org/1999/xhtml"/>'],
  ['missing-namespace', '<svg width="64" height="64"><rect width="64" height="64"/></svg>'],
  ['undefined-entity', '<svg xmlns="http://www.w3.org/2000/svg">&missing;</svg>'],
  ['empty', ''],
] as const;

test('corrupt persisted level 1 SVGs paint an empty image through the engine', async () => {
  test.setTimeout(180_000);
  const out = process.env.PM_MEDIA_EVIDENCE ? path.join(process.env.PM_MEDIA_EVIDENCE, 'broken-svg') : await mkdtemp(path.join(os.tmpdir(), 'pm-broken-svg-'));
  await mkdir(out, { recursive: true });
  const frames: number[][][] = [];
  try {
    for (const native of [false, true]) {
      const app = await launchApp({ env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' } });
      const captured: number[][] = [];
      try {
        await app.openEditor();
        for (const [name, source] of sources) {
          const facts = await app.page.evaluate(async ({ name, source }) => {
            const PM = (window as any).PM;
            PM.replaceProject(PM.mkProject({ w: 64, h: 64, fps: 30, dur: 1, bg: '#526e83', name }));
            PM.ProjectsScreen.hide();
            const id = `broken-${name}`, storageKey = `${id}.svg`;
            const blob = new Blob([source], { type: 'image/svg+xml' });
            const stored = await PM.MediaStore.put(storageKey, blob, { storageKey, type: 'image/svg+xml' });
            const contentHash = PM.MediaStore.storedRecord?.(blob)?.hash;
            PM.proj.assets[id] = { id, name: `${name}.svg`, storageKey, kind: 'image', format: 'svg', w: 64, h: 64, persisted: true, ...(contentHash ? { contentHash } : {}) };
            PM.proj.layers.push(PM.mkLayer('image', { d: { asset: id, w: 64, h: 64 }, p: { 'position.x': 32, 'position.y': 32 } }));
            await PM.HostMedia?.flushBindings(PM.proj);
            PM.touch(); PM.assets.clear(); await PM.assets.restoreProject(PM.proj); await PM.prepareFrame(0);
            return { stored: Boolean(stored), live: PM.assets.map.has(id) };
          }, { name, source });
          expect(facts).toEqual({ stored: true, live: false });
          if (native) await expect.poll(() => app.page.evaluate(() => {
            const s = (window as any).PM.EngineConnection.status(); return { route: s.route, reason: s.routeReason };
          })).toEqual({ route: 'engine', reason: null });
          await app.page.evaluate(() => {
            const w = window as any; w.__brokenJobs = [];
            w.__brokenOff = w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => w.__brokenJobs.push(s));
          });
          const output = path.join(out, `${native ? 'native' : 'legacy'}-${name}.png`);
          await app.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, output);
          const result = await app.page.evaluate(async () => (window as any).PM.Export.run({ format: 'still', w: 64, h: 64, alpha: true, mblur: false }));
          const jobs = await app.page.evaluate(() => { const w = window as any; w.__brokenOff(); return w.__brokenJobs; });
          await writeFile(path.join(out, `${native ? 'native' : 'legacy'}-${name}.json`), JSON.stringify({ facts, result, jobs, diagnostics: app.diagnostics }, null, 2));
          if (native) {
            expect(jobs.some((j: any) => j.state === 'done' && !j.routed), name).toBe(true);
            expect(jobs.some((j: any) => j.routed), name).toBe(false);
          }
          expect(result, `${native ? 'native' : 'legacy'} ${name}`).toEqual({ cancelled: false });
          captured.push(await app.page.evaluate(async bytes => {
            const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
            const canvas = new OffscreenCanvas(64, 64), ctx = canvas.getContext('2d')!;
            ctx.drawImage(image, 0, 0); image.close(); return [...ctx.getImageData(0, 0, 64, 64).data];
          }, [...await readFile(output)]));
        }
      } finally { await app.close(); }
      frames.push(captured);
    }
    // Compare decoded RGBA instead of encoder metadata or compression choices.
    const metrics = sources.map(([name], i) => {
      const a = frames[0]![i]!, b = frames[1]![i]!;
      return { name, max: a.reduce((m, v, p) => Math.max(m, Math.abs(v - b[p]!)), 0), alphaMax: Math.max(...b.filter((_, i) => i % 4 === 3)) };
    });
    await writeFile(path.join(out, 'metrics.json'), JSON.stringify(metrics, null, 2));
    for (const m of metrics) expect(m).toEqual({ name: m.name, max: 0, alphaMax: 0 });
  } finally { if (!process.env.PM_MEDIA_EVIDENCE) await rm(out, { recursive: true, force: true }); }
});
