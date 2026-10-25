/* Broken persisted SVG assets have the same empty paint as today's <img>.
   Documents roxmltree rejects but Chromium's libxml2 paints (DTD external
   subsets, parameter and external entities) must not be mistaken for broken
   ones: the engine draws SVG images natively (docs/engine/vector.md) and
   must paint them as today. */
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

// Chromium paints these (measured with Blob <img> + drawImage in Electron
// 44); roxmltree reports errors for all of them.
const R = '<rect width="64" height="64" fill="red"/>';
const NS = 'xmlns="http://www.w3.org/2000/svg" width="64" height="64"';
const painted = [
  ['svg11-doctype-nbsp', `<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg ${NS}>${R}<text>&nbsp;&copy;</text></svg>`],
  ['system-doctype-entity', `<!DOCTYPE svg SYSTEM "svg.dtd"><svg ${NS}>${R}<text>&foo;</text></svg>`],
  ['external-entity', `<!DOCTYPE svg [<!ENTITY e SYSTEM "e.xml">]><svg ${NS}>${R}&e;</svg>`],
  ['parameter-entity', `<!DOCTYPE svg [<!ENTITY % a "<!ENTITY b 'x'>"> %a;]><svg ${NS}>${R}<text>&b;</text></svg>`],
] as const;
const all = [...sources, ...painted];

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
        for (const [name, source] of all) {
          const valid = painted.some(([n]) => n === name);
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
          // Today's <img> loads only the documents Chromium paints.
          expect(facts).toEqual({ stored: true, live: valid });
          const status = () => app.page.evaluate(() => {
            const s = (window as any).PM.EngineConnection.status(); return { route: s.route, reason: s.routeReason };
          });
          // Broken or valid, the engine draws it: nothing is routed.
          if (native) await expect.poll(status).toEqual({ route: 'engine', reason: null });
          await app.page.evaluate(() => {
            const w = window as any; w.__brokenJobs = [];
            w.__brokenOff = w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => w.__brokenJobs.push(s));
          });
          const output = path.join(out, `${native ? 'native' : 'legacy'}-${name}.png`);
          await app.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, output);
          const result = await app.page.evaluate(async () => (window as any).PM.Export.run({ format: 'still', w: 64, h: 64, alpha: true, mblur: false }));
          const jobs = await app.page.evaluate(() => { const w = window as any; w.__brokenOff(); return w.__brokenJobs; });
          await writeFile(path.join(out, `${native ? 'native' : 'legacy'}-${name}.json`), JSON.stringify({ facts, result, jobs, diagnostics: app.diagnostics }, null, 2));
          // The job engine exports it, never today's exporter; the export
          // shows today's paint.
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
    const metrics = all.map(([name], i) => {
      const a = frames[0]![i]!, b = frames[1]![i]!;
      return { name, max: a.reduce((m, v, p) => Math.max(m, Math.abs(v - b[p]!)), 0), alphaMax: Math.max(...b.filter((_, i) => i % 4 === 3)), center: b.slice((32 * 64 + 32) * 4, (32 * 64 + 32) * 4 + 4) };
    });
    await writeFile(path.join(out, 'metrics.json'), JSON.stringify(metrics, null, 2));
    for (const m of metrics) {
      const valid = painted.some(([n]) => n === m.name);
      expect(m).toEqual({ name: m.name, max: 0, alphaMax: valid ? 255 : 0, center: valid ? [255, 0, 0, 255] : [0, 0, 0, 0] });
    }
  } finally { if (!process.env.PM_MEDIA_EVIDENCE) await rm(out, { recursive: true, force: true }); }
});
