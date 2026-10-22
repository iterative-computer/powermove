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
 * Opt in (one Playwright worker, under the Electron lock):
 *   PM_TEXT_TYPING_MEASURE=1 pm-electron-lock bunx playwright test e2e/native-engine-text-typing.spec.ts --workers=1
 * PM_TEXT_TYPING_DIAGNOSE=1 exercises the same input/presentation path under
 * contention, with bounded per-key failure diagnostics. It never qualifies
 * the 33 ms latency target. Measurement preconditions fail before launch;
 * wait for quiet outside the Electron lock.
 * Results go to $PM_TEXT_TYPING_OUT (JSON lines; default
 * /tmp/powermove-text-typing.jsonl), never into the repository.
 * PM_TEXT_TYPING_PAIRED=1 runs today's renderer beside the native renderer
 * for every scenario. Legacy completion is the first successful GL.render
 * with the changed layer text, sampled by the same RAF polling loop.
 */
import { appendFile, statfs } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp } from './helpers/app';

const DIAGNOSTIC = process.env.PM_TEXT_TYPING_DIAGNOSE === '1';
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
  for (const mode of MODES) test(`canvas text typing: ${scenario.name} (${mode})`, async ({}, testInfo) => {
    test.skip(!MEASURE && !DIAGNOSTIC, 'set PM_TEXT_TYPING_MEASURE=1 or PM_TEXT_TYPING_DIAGNOSE=1');
    test.setTimeout(180_000);
    // The Playwright command owns the Electron slot. Never occupy it while
    // waiting for a quiet machine before Electron has even launched.
    if (!DIAGNOSTIC && os.loadavg()[0] > 8) throw new Error('Machine load exceeds 8 before Electron launch; latency is unmeasured. Queue for a quiet machine outside the Electron lock, or use PM_TEXT_TYPING_DIAGNOSE=1 for responsiveness only.');
    const disk = await statfs('/System/Volumes/Data');
    if (disk.bavail * disk.bsize < 40 * 1024 ** 3) throw new Error('Disk below required 40 GiB before typing launch');
    const loadBefore = os.loadavg();
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: mode === 'native' ? '1' : '0', POWERMOVE_ENGINE_FRAME_SOURCE: 'document', POWERMOVE_BRIDGE_TRACE: '1' } });
    const mainConsole: string[] = [];
    session.app.on('console', message => { mainConsole.push(message.text()); if (mainConsole.length > 200) mainConsole.shift(); });
    const captureState = async (label: string) => {
      const bounded = async (operation: Promise<unknown>) => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        try { return await Promise.race([operation, new Promise(resolve => { timer = setTimeout(() => resolve({ unavailable: 'diagnostic request exceeded 5 seconds' }), 5_000); })]); }
        catch (error) { return { unavailable: String(error) }; }
        finally { clearTimeout(timer); }
      };
      const [renderer, engine] = await Promise.all([
        bounded(session.page.evaluate(() => {
          const w = window as any, PM = w.PM, viewer = PM?.Kernel.services.get('viewer');
          const session = PM?.EngineConnection?.session();
          const connection = session?.options?.connection;
          const map = (value: any) => value instanceof Map ? [...value.keys()] : null;
          return {
            status: PM?.EngineConnection?.status(), document: PM?.EngineDoc?.stats?.(),
            frame: viewer?.engineFrame?.(), transport: PM?.EngineRender?.stats,
            pendingKey: w.__typing?.pending, lastFrames: w.__typing?.frames?.slice(-24), lastBatches: w.__typing?.batches?.slice(-24),
            session: session ? { id: session.id, revision: session.revision, applied: session.applied, resyncPending: session.resyncPending, queuedBatches: session.queued?.length, fallbackWaiting: map(session.fallbackWaiting) } : null,
            connection: connection ? { ready: connection.ready, closed: connection.closed, outstandingIds: map(connection.outstanding?.pending), outstandingCount: connection.outstanding?.pending?.size, heldPatches: connection.heldPatches?.length, errors: connection.errors } : null,
          };
        })),
        bounded(session.app.evaluate(async () => await (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive') ?? null)),
      ]);
      await testInfo.attach(`typing-${label}-state`, { body: Buffer.from(JSON.stringify({ diagnosticOnly: DIAGNOSTIC, load: os.loadavg(), renderer, engine, mainConsole, rendererConsole: session.diagnostics }, null, 2)), contentType: 'application/json' });
      await session.page.screenshot({ path: testInfo.outputPath(`typing-${label}-frame.png`), timeout: 5_000 }).catch(() => undefined);
    };
    try {
      const { page } = session;
      const displayBefore = await session.app.evaluate(({}, harnessPath) => process.getBuiltinModule('module').createRequire(harnessPath)(harnessPath).displayState(), process.env.POWERMOVE_PM_BRIDGE_HARNESS ?? path.join(os.homedir(),'Library/Caches/powermove-engine/pm_bridge_harness.node'));
      if (!DIAGNOSTIC && (displayBefore.asleep || displayBefore.locked)) throw new Error('Display asleep or locked; typing row is unmeasured');
      await session.openEditor();
      if (mode === 'native') {
        await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
        await page.evaluate(() => (window as any).PM.EngineConnection.setTrace(true));
      }
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
        w.__typing = { pending: null as null | { t: number; revision: number }, samples: [] as number[], keys: [] as any[], batches: [] as any[], frames: [] as any[], failures: 0 };
        if (mode === 'native') {
          w.PM.EngineDoc.subscribe((b:any)=>w.__typing.batches.push({at:performance.now(),revision:b.revision}));
          viewer.enginePresentation.onPresented((f:any)=>w.__typing.frames.push({at:performance.now(),seq:f.seq,revision:f.revision,rafMs:f.rafMs}));
        }
        document.addEventListener('keydown', (e) => {
          const frame = mode === 'native' ? viewer.engineFrame() : { revision: legacyRevision };
          w.__typing.pending = { t: e.timeStamp, handled:performance.now(), revision: mode === 'native' ? (w.PM.EngineDoc.barrier?.revision ?? -1) : frame?.revision ?? -1, text: w.PM.resolveContent(w.PM.L(id), w.PM.time)?.text };
        }, { capture: true });
        const tick = (now: number) => {
          const p = w.__typing.pending;
          const frame = mode === 'native' ? viewer.engineFrame() : { revision: legacyRevision };
          if (p && frame && frame.revision > p.revision) {
            w.__typing.samples.push(now - p.t);
            w.__typing.keys.push({...p,presentedRaf:now,shownRevision:frame.revision});
            w.__typing.pending = null;
          }
          requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }, { mode, id });
      await captureState('before-input');
      const letters = 'abcdefghijklmnopqrstuvwxyz ';
      // A keystroke whose frame never comes is a miss, counted (never
      // dropped from the percentiles silently): the gate wants every one.
      let missed = 0;
      for (let i = 0; i < KEYS; i++) {
        await page.keyboard.type(letters[i % letters.length]!);
        const drawn = await page.waitForFunction(() => (window as any).__typing.pending === null, undefined, { timeout: 5_000 }).then(() => true, () => false);
        if (!drawn) {
          missed++;
          await captureState(`stalled-key-${i}`);
          throw new Error(`Typing key ${i + 1} did not produce a newer presented frame within 5 seconds; renderer/engine state and last frame are attached.`);
        }
        await page.waitForTimeout(60);
      }
      const result = await page.evaluate(async (mode) => {
        const w = window as any;
        const PM = w.PM;
        const viewer = PM.Kernel.services.get('viewer');
        return { details:{keys:w.__typing.keys,batches:w.__typing.batches,frames:w.__typing.frames,hostOffsetUs:PM.EngineConnection?.status().hostOffsetUs,trace:mode === 'native' ? await viewer.enginePresentation.takeTrace() : null}, samples: w.__typing.samples as number[], route: PM.EngineConnection?.status()?.route ?? 'legacy', text: PM.resolveContent(PM.L(PM.sel.layers?.[0] ?? ''), PM.time)?.text };
      }, mode);
      await captureState('after-input');
      await page.keyboard.press('Escape');
      const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive') ?? null);
      const displayAfter = await session.app.evaluate(({}, harnessPath) => process.getBuiltinModule('module').createRequire(harnessPath)(harnessPath).displayState(), process.env.POWERMOVE_PM_BRIDGE_HARNESS ?? path.join(os.homedir(),'Library/Caches/powermove-engine/pm_bridge_harness.node'));
      const samples = result.samples;
      const value = {
        scenario: scenario.name,
        mode,
        keys: KEYS,
        measured: samples.length,
        missed,
        p50: samples.length ? Math.round(percentile(samples, 50) * 100) / 100 : null,
        p95: samples.length ? Math.round(percentile(samples, 95) * 100) / 100 : null,
        p99: samples.length ? Math.round(percentile(samples, 99) * 100) / 100 : null,
        max: samples.length ? Math.round(Math.max(...samples) * 100) / 100 : null,
        route: result.route,
        layerFailures: report?.presenter?.layerFailures ?? null,
        renderErrors: report?.presenter?.renderErrors ?? null,
        fonts: report?.fonts ?? null,
      };
      await appendFile(OUT, `${JSON.stringify({ row: DIAGNOSTIC ? 'canvas-text-typing-responsiveness' : 'canvas-text-typing', diagnosticOnly: DIAGNOSTIC, at: new Date().toISOString(), cpu: os.cpus()[0]?.model ?? null, displayBefore,displayAfter,loadBefore, load: os.loadavg(), invalid:os.loadavg()[0]>8?'machine load exceeded 8':null, value, details:result.details, engineTrace:report?.presentTrace??null })}\n`);
      console.log(`[${DIAGNOSTIC ? 'responsiveness' : 'measure'}] canvas text typing: ${JSON.stringify(value)}`);
      if (mode === 'native') {
        expect(result.route).toBe('engine');
        expect(value.layerFailures).toBe(0);
        expect(value.renderErrors).toBe(0);
      }
      if (!DIAGNOSTIC) {
        expect(displayAfter.asleep || displayAfter.locked).toBe(false);
        expect(os.loadavg()[0], 'machine load exceeded 8; retry this pair').toBeLessThanOrEqual(8);
      }
      expect(result.text).toBe(scenario.d.text + Array.from({ length: KEYS }, (_, i) => letters[i % letters.length]).join(''));
      expect(missed).toBe(0);
      expect(samples.length).toBe(KEYS);
      if (!DIAGNOSTIC) expect(value.p95!).toBeLessThanOrEqual(33);
    } catch (error) {
      await captureState('failure');
      throw error;
    } finally {
      await session.close();
    }
  });
}
