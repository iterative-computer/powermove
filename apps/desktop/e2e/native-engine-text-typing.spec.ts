/*
 * The "Canvas text typing: p95 ≤ 33 ms" row (plan section 25, Phase 3a)
 * measured end to end in the app with "Use New Engine" on and the engine
 * drawing the text (docs/engine/text-render.md, "Measurements").
 *
 * A 1920 × 1080 project with one text layer routes to the engine (the
 * renderer has fonts: no legacy frame). The test double-clicks the layer to
 * edit it on the canvas and types one character at a time, waiting for each
 * keystroke to reach the screen. A keystroke's latency is from its
 * `keydown` event's timestamp (in the page) to the animation frame in which
 * the viewer first shows an engine frame of a newer document revision than
 * the one on screen at the keystroke: input, the edit, the patch to the
 * engine, layout and glyph rendering on its render thread, the frame
 * bridge, and the viewer's presentation. Display scan-out is not included.
 *
 * Opt in (one Electron at a time):
 *   PM_TEXT_TYPING_MEASURE=1 pm-electron-lock bunx playwright test e2e/native-engine-text-typing.spec.ts --workers=1
 * Results go to $PM_TEXT_TYPING_OUT (JSON lines; default
 * /tmp/powermove-text-typing.jsonl), never into the repository.
 * PM_TEXT_TYPING_PAIRED=1 runs today's renderer beside the native renderer
 * for every scenario. Legacy completion is the first successful GL.render
 * with the changed layer text, sampled by the same RAF polling loop.
 */
import { appendFile } from 'node:fs/promises';
import os from 'node:os';

import { expect, test } from '@playwright/test';

import { launchApp } from './helpers/app';

const MEASURE = process.env.PM_TEXT_TYPING_MEASURE === '1';
const MODES = process.env.PM_TEXT_TYPING_PAIRED === '1' ? ['legacy', 'native'] as const : ['native'] as const;
const OUT = process.env.PM_TEXT_TYPING_OUT ?? '/tmp/powermove-text-typing.jsonl';
const KEYS = Number(process.env.PM_TEXT_TYPING_KEYS ?? 60);

const percentile = (values: number[], p: number): number => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)]!;
};

for (const scenario of [
  { name: 'title 96 px', d: { text: 'Typing', font: 'SF Pro Display', weight: 700, size: 96, color: '#FFFFFF', align: 'center' } },
  { name: 'paragraph 48 px in a 1400 px box', d: { text: 'Motion design brings graphics to life.', font: 'SF Pro Display', weight: 400, size: 48, color: '#FFFFFF', align: 'left', boxWidth: 1400, paragraph: true } },
  { name: 'system-ui 24 px', d: { text: 'Caption', font: 'system-ui', weight: 400, size: 24, color: '#FFFFFF', align: 'center' } },
]) {
  for (const mode of MODES) test(`canvas text typing: ${scenario.name} (${mode})`, async () => {
    test.skip(!MEASURE, 'set PM_TEXT_TYPING_MEASURE=1');
    test.setTimeout(600_000);
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: mode === 'native' ? '1' : '0', POWERMOVE_ENGINE_FRAME_SOURCE: 'document' } });
    try {
      const { page } = session;
      await session.openEditor();
      if (mode === 'native') await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
      const id = await page.evaluate((d) => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Typing', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#101010' });
        const layer = PM.mkLayer('text', { d, p: { 'position.x': 960, 'position.y': 480 } }, project);
        project.layers = [layer];
        PM.replaceProject(project);
        PM.selectLayers(layer.id);
        const viewer = PM.Kernel.services.get('viewer');
        PM.Kernel.services.get('tool').setTool('select');
        viewer.fit = true;
        viewer.layout();
        PM.hist.clear();
        PM.invalidate();
        return layer.id;
      }, scenario.d);
      // The composition draws in the engine, text included.
      if (mode === 'native') await expect.poll(async () => {
        const s = await page.evaluate(() => {
          const PM = (window as any).PM;
          return { route: PM.EngineConnection?.status()?.route ?? 'legacy', presenting: !!PM.Kernel.services.get('viewer').enginePresentation?.presenting };
        });
        return `${s.route}/${s.presenting}`;
      }, { timeout: 60_000 }).toBe('engine/true');
      const center = await page.evaluate((layerId) => {
        const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'), b = viewer.worldBounds(PM.L(layerId), PM.time);
        const r = document.querySelector('#stage-inner')!.getBoundingClientRect();
        return { x: r.x + b.cx * viewer.shown, y: r.y + b.cy * viewer.shown };
      }, id);
      await page.mouse.dblclick(center.x, center.y);
      const editor = page.getByRole('textbox', { name: 'Edit text on canvas' });
      await expect(editor).toBeFocused();
      await page.keyboard.press('End');
      // The page side: each keydown's timestamp and the revision on screen
      // then; every animation frame checks for a newer engine frame.
      await page.evaluate(({ mode, id }) => {
        const w = window as any;
        const viewer = w.PM.Kernel.services.get('viewer');
        let legacyRevision = 0;
        const draw = w.PM.GL.render;
        if (mode === 'legacy') w.PM.GL.render = function(...args: unknown[]) {
          const result = draw.apply(this, args);
          const p = w.__typing.pending;
          if (result !== false && p && w.PM.resolveContent(w.PM.L(id), w.PM.time)?.text !== p.text) legacyRevision++;
          return result;
        };
        w.__typing = { pending: null as null | { t: number; revision: number }, samples: [] as number[], failures: 0 };
        document.addEventListener('keydown', (e) => {
          const frame = mode === 'native' ? viewer.engineFrame() : { revision: legacyRevision };
          w.__typing.pending = { t: e.timeStamp, revision: frame?.revision ?? -1, text: w.PM.resolveContent(w.PM.L(id), w.PM.time)?.text };
        }, { capture: true });
        const tick = (now: number) => {
          const p = w.__typing.pending;
          const frame = mode === 'native' ? viewer.engineFrame() : { revision: legacyRevision };
          if (p && frame && frame.revision > p.revision) {
            w.__typing.samples.push(now - p.t);
            w.__typing.pending = null;
          }
          requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }, { mode, id });
      const letters = 'abcdefghijklmnopqrstuvwxyz ';
      // A keystroke whose frame never comes is a miss, counted (never
      // dropped from the percentiles silently): the gate wants every one.
      let missed = 0;
      for (let i = 0; i < KEYS; i++) {
        await page.keyboard.type(letters[i % letters.length]!);
        const drawn = await page.waitForFunction(() => (window as any).__typing.pending === null, undefined, { timeout: 5_000 }).then(() => true, () => false);
        if (!drawn) {
          missed++;
          await page.evaluate(() => { (window as any).__typing.pending = null; });
        }
        await page.waitForTimeout(60);
      }
      const result = await page.evaluate(() => {
        const w = window as any;
        const PM = w.PM;
        return { samples: w.__typing.samples as number[], route: PM.EngineConnection?.status()?.route ?? 'legacy', text: PM.L(PM.sel.layers?.[0] ?? '')?.d?.text };
      });
      await page.keyboard.press('Escape');
      const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive') ?? null);
      const samples = result.samples;
      const value = {
        scenario: scenario.name,
        mode,
        keys: KEYS,
        measured: samples.length,
        missed,
        p50: samples.length ? Math.round(percentile(samples, 50) * 100) / 100 : null,
        p95: samples.length ? Math.round(percentile(samples, 95) * 100) / 100 : null,
        max: samples.length ? Math.round(Math.max(...samples) * 100) / 100 : null,
        route: result.route,
        layerFailures: report?.presenter?.layerFailures ?? null,
        renderErrors: report?.presenter?.renderErrors ?? null,
        fonts: report?.fonts ?? null,
      };
      await appendFile(OUT, `${JSON.stringify({ row: 'canvas-text-typing', at: new Date().toISOString(), cpu: os.cpus()[0]?.model ?? null, load: os.loadavg(), value })}\n`);
      console.log(`[measure] canvas text typing: ${JSON.stringify(value)}`);
      if (mode === 'native') expect(result.route).toBe('engine');
      expect(missed).toBe(0);
      expect(samples.length).toBe(KEYS);
      expect(value.p95!).toBeLessThanOrEqual(33);
    } finally {
      await session.close();
    }
  });
}
