/** C1 reads the private corpus in place. No private source, media, screenshots,
 * operational notes, or results belong in Git. All app state is disposable. */
import { cp, mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { Page } from 'playwright';
import { expect, launchApp, test } from './helpers/app';

const CORPUS = process.env.PM_PRIVATE_SETUP_DIR ?? path.join(os.homedir(), 'Library/Application Support/Powermove Engine Corpus/private-setup');
const EVIDENCE = path.join(os.tmpdir(), 'powermove-private-setup-compat');
const REGISTRIES = ['effects', 'transitions', 'layerTypes', 'commands', 'panels', 'themes', 'keybindings', 'inspectorSections'] as const;

async function facts(page: Page, ids: string[]) {
  return page.evaluate(({ ids, registries }) => {
    const PM = (window as any).PM, kernel = PM.Kernel, api = kernel.api('corpus');
    return ids.map((id) => {
      const record = api.extensions.list().find((r: any) => r.id === id);
      const contributions = Object.fromEntries(registries.map((name) => [name, (kernel[name]?.ownerEntries?.(id) ?? []).map((e: any) => e.id).sort()]));
      return { id, active: kernel.loader.activeIds().includes(id), enabled: record?.enabled, health: record?.health?.state, error: record?.health?.error ?? record?.health?.message ?? record?.error ?? null, contributions };
    });
  }, { ids, registries: [...REGISTRIES] });
}

async function exercise(page: Page, ids: string[]) {
  return page.evaluate(async (ids) => {
    const PM = (window as any).PM, kernel = PM.Kernel, api = kernel.api('corpus');
    const panels = [];
    for (const entry of kernel.panels.entries().filter((e: any) => ids.includes(e.ownerId))) {
      try {
        api.panels.open(entry.id, 'right');
        await new Promise((r) => setTimeout(r, 80));
        const root = document.getElementById(`panel-${entry.id}`);
        panels.push({ owner: entry.ownerId, id: entry.id, mounted: !!root, controls: root ? [...root.querySelectorAll('button,input,select,textarea,canvas')].map((e: any) => [e.tagName, e.getAttribute('aria-label') ?? e.getAttribute('type') ?? '']) : [] });
        api.panels.close(entry.id);
      } catch (error) { panels.push({ owner: entry.ownerId, id: entry.id, error: String(error) }); }
    }
    const commands = [];
    // Open/read commands exercise their actual public registrations. Commands
    // that record, download, import or mutate a project are not blind-fired.
    for (const entry of kernel.commands.entries().filter((e: any) => ids.includes(e.ownerId))) {
      if (!/(?:\.open|\.read)$/.test(entry.id)) continue;
      try { await api.commands.run(entry.id); commands.push({ owner: entry.ownerId, id: entry.id, ok: true }); }
      catch (error) { commands.push({ owner: entry.ownerId, id: entry.id, ok: false, error: String(error) }); }
    }
    await api.commands.run('private-extension-01.apply');
    await new Promise((r) => setTimeout(r, 150));
    const theme = { active: api.theme.active(), classes: document.body.className, tools: document.querySelectorAll('.toolstrip button').length, stage: !!document.querySelector('#stage'), timeline: !!document.querySelector('#panel-timeline') };
    const tool = kernel.services.get('tool');
    const tools = [];
    for (const id of ['toolHand', 'toolZoom', 'toolSelect']) {
      if (!api.commands.has(id)) { tools.push([id, 'missing']); continue; }
      await api.commands.run(id); tools.push([id, tool?.tool]);
    }
    const gradient = kernel.effects.entries().find((e: any) => e.id === 'gradient')?.ownerId ?? null;
    return { panels, commands, theme, tools, gradient, rasterWrapped: !!PM.RasterCompat?.wrapped(), legacyPm: kernel.api('private-extension-38').host.pm === PM };
  }, ids);
}

async function frame(page: Page, time: number, width: number, height: number) {
  return page.evaluate(async ({ time, width, height }) => {
    const PM = (window as any).PM, api = PM.Kernel.api('corpus');
    const progress = (window as any).__c1Frame = { time, phase: 'prepare' };
    PM.pause(); PM.setTime(time, { force: true });
    await PM.prepareFrame?.(time);
    progress.phase = 'bindings';
    await PM.HostMedia?.flushBindings?.();
    progress.phase = 'render';
    const canvas = await api.render.renderFrameToAsync(time, width, height, { mblur: true, mbSamples: 16 });
    if (!canvas) throw new Error(`C1: no frame at ${time}`);
    progress.phase = 'pixels';
    const bytes = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
    // Base64 avoids huge number[] transfers for the private full-size frames.
    let raw = ''; for (let at = 0; at < bytes.length; at += 32768) raw += String.fromCharCode(...bytes.subarray(at, at + 32768));
    return { width: canvas.width, height: canvas.height, rgba: btoa(raw), png: canvas.toDataURL('image/png') };
  }, { time, width, height });
}

async function wrapperProject(page: Page) {
  return page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('corpus');
    const p = PM.mkProject({ name: 'C1 raster wrappers', w: 480, h: 270, fps: 30, dur: 2, bg: '#203040' });
    const counter = PM.mkLayer('text', { from: 0, dur: 2, d: { text: 'Count', font: 'Helvetica', size: 36, color: '#FFFFFF' } }, p);
    counter.id = 'counter'; counter.p['position.x'].v = 160; counter.p['position.y'].v = 60;
    const count = PM.mkEffect('private-extension-25'); count.p.value = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(2, 100)] }); count.p.decimals.v = 0; counter.fx = [count];
    const typed = PM.mkLayer('text', { from: 0, dur: 2, d: { text: 'Wrapper contract', font: 'Helvetica', size: 32, color: '#FFFFFF' } }, p);
    typed.id = 'typed'; typed.p['position.x'].v = 240; typed.p['position.y'].v = 120;
    const fx = PM.mkEffect('typewriter'); fx.p.progress = PM.P(0, { kf: [PM.KF(0, 0), PM.KF(2, 100)] }); typed.fx = [fx];
    const points = [[80, 220], [140, 150], [340, 270], [400, 200]].map(([x, y], i) => {
      const l = PM.mkLayer('shape', { from: 0, dur: 2, d: { shape: 'ellipse', w: 4, h: 4, color: '#FF641E', stroke: 0 } }, p);
      l.id = `point-${i}`; l.p['position.x'].v = x; l.p['position.y'].v = y; return l;
    });
    const linked = PM.mkLayer('shape', { from: 0, dur: 2, d: { shape: 'rect', w: 440, h: 140, color: '#00000000', stroke: 8, strokeColor: '#FF641E' } }, p);
    linked.id = 'linked'; linked.p['position.x'].v = 240; linked.p['position.y'].v = 200;
    linked.d.linkedCurve = { version: 1, kind: 'curve', points: points.map((l: any) => l.id) };
    p.layers = [counter, typed, linked, ...points]; PM.replaceProject(p); PM.Projects.put(PM.proj);
    return [counter, typed, linked].map((layer: any) => {
      const r = api.render.raster(layer, 1, .5);
      return { id: layer.id, sync: !!r && !(r instanceof Promise), wrapper: layer.id === 'counter' ? String(r?.key).startsWith('private-extension-25:') : layer.id === 'typed' ? String(r?.key).includes('typewriter') : String(r?.key).startsWith('private-extension-18:'), geometry: r && [r.w, r.h, r.anchorX, r.anchorY, r.cv.width, r.cv.height] };
    });
  });
}

function measure(a: Buffer, b: Buffer) {
  let max = 0, over1 = 0, values = 0;
  if (a.length !== b.length) throw new Error('C1 frame sizes differ');
  for (let i = 0; i < a.length; i++) if (i % 4 !== 3) { const d = Math.abs(a[i]! - b[i]!); max = Math.max(max, d); over1 += Number(d > 1); values++; }
  return { max, over1, values, pass: max <= 4 && over1 <= values * .001 };
}

test('C1 private setup: all extensions, contribution winners, host UI, wrappers and real project frames', async ({ session }, testInfo) => {
  test.setTimeout(1_800_000);
  const project = path.join(CORPUS, 'projects/private-project.pmv');
  const present = await stat(project).then(() => true, () => false);
  test.skip(!present, 'C1 private setup corpus absent: set PM_PRIVATE_SETUP_DIR or install the external private-setup corpus');
  const relativeOutput = path.relative(path.resolve(__dirname, '../../..'), testInfo.outputDir);
  if (relativeOutput !== '..' && !relativeOutput.startsWith(`..${path.sep}`) && !path.isAbsolute(relativeOutput)) {
    throw new Error('C1 private artifacts must stay outside the public repository. Run with --config playwright.private-corpus.config.ts.');
  }

  await session.close();
  await mkdir(EVIDENCE, { recursive: true });
  const ids = [];
  for (const entry of await readdir(path.join(CORPUS, 'extensions'), { withFileTypes: true })) if (entry.isDirectory() && await stat(path.join(CORPUS, 'extensions', entry.name, 'manifest.json')).then(() => true, () => false)) ids.push(entry.name);
  ids.sort(); expect(ids.length).toBeGreaterThan(0);
  const results: Record<string, any> = {};
  const captures: Array<{ time: number; baseline: string; png: string; width: number; height: number }> = [];
  const profiles: string[] = [];
  try {
    for (const engine of [false, true]) {
      const profile = await mkdtemp(path.join(os.tmpdir(), 'powermove-c1-')); profiles.push(profile);
      // Prime only the test's desired appearance state. The corpus omits
      // saved settings, and the source has an explicit one-time recovery
      // request. Acknowledge that request in this isolated
      // profile before activation; never alter its source or error policy.
      const prime = await launchApp({ userData: profile, env: {} });
      try {
        await prime.openEditor();
        await prime.page.evaluate(async () => { const PM = (window as any).PM; PM.Kernel.api('private-extension-01').storage.set('recovery-acknowledged', true); await (window as any).__powermoveTestBridge.store.flush(); });
      } finally { await prime.close(); }
      for (const id of ids) await cp(path.join(CORPUS, 'extensions', id), path.join(profile, 'extensions', id), { recursive: true });
      const app = await launchApp({ userData: profile, env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0' } });
      const mode = engine ? 'engine' : 'today';
      try {
        await app.openEditor();
        console.log(`C1 ${mode}: waiting for all extension activations`);
        await app.page.waitForFunction((ids) => {
          const PM = (window as any).PM;
          if (!PM?.Kernel?.loader) return false;
          const active = PM.Kernel.loader.activeIds();
          const records = PM.Kernel.api('corpus').extensions.list();
          return ids.every((id) => id === 'private-extension-32'
            ? records.find((r: any) => r.id === id)?.health?.state === 'build-error'
            : active.includes(id));
        }, ids, { timeout: 90_000, polling: 100 });
        results[mode] = { extensions: await facts(app.page, ids), frames: [] };
        await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
        results[mode].ui = await exercise(app.page, ids);
        console.log(`C1 ${mode}: UI exercised; checking private raster wrappers`);
        results[mode].wrappers = { surfaces: await wrapperProject(app.page), frames: [] };
        for (const time of [.2, 1.2]) {
          const rendered = await frame(app.page, time, 480, 270);
          const png = path.join(EVIDENCE, `${mode}-wrappers-${time}.png`);
          await writeFile(png, Buffer.from(rendered.png.split(',')[1]!, 'base64'));
          const raw = path.join(EVIDENCE, `today-wrappers-${time}.rgba`);
          if (!engine) await writeFile(raw, Buffer.from(rendered.rgba, 'base64'));
          results[mode].wrappers.frames.push({ time, png, ...(engine ? measure(await readFile(raw), Buffer.from(rendered.rgba, 'base64')) : {}) });
        }
        await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
        await app.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, project);
        console.log(`C1 ${mode}: opening private project`);
        await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
        await app.page.evaluate(() => {
          const w = window as any;
          w.__c1OpenDone = false; w.__c1Toasts = [];
          const toast = w.PM.toast;
          w.PM.toast = (...args: any[]) => { w.__c1Toasts.push(String(args[0])); return toast(...args); };
          void w.PM.openProject().then(() => { w.__c1OpenDone = true; });
        });
        try {
          await expect.poll(() => app.page.evaluate(() => (window as any).PM.proj.layers.length), { timeout: 180_000, message: 'C1: original private project must load all its layers' }).toBeGreaterThan(0);
        } catch (error) {
          results[mode].openWaitError = String(error);
          // A busy renderer can finish the import after the test's deadline.
          // Keep the timeout as a failed gate, but collect its real pixels if
          // the original document is now available rather than losing evidence.
          if (await app.page.evaluate(() => (window as any).PM.proj.layers.length) === 0) throw error;
        }
        // Layer installation precedes the file fingerprint and workspace
        // restore. Wait for the real open operation before exercising the
        // loaded document; a layer count alone is not an open-completion gate.
        try {
          await expect.poll(() => app.page.evaluate(() => (window as any).__c1OpenDone), { timeout: 600_000, message: 'C1: original project open must finish before captures' }).toBe(true);
        } catch (error) { results[mode].openCompletionError = String(error); throw error; }
        if (engine) expect(await app.page.evaluate(() => !!(window as any).PM.EngineRender), 'C1: engine facade remains installed').toBe(true);
        const setup = await app.page.evaluate(() => { const PM = (window as any).PM; return { dur: PM.proj.dur, fps: PM.proj.fps, w: PM.proj.w, h: PM.proj.h, typewriters: PM.proj.layers.filter((l: any) => l.fx?.some((f: any) => f.type === 'typewriter')).map((l: any) => ({ from: l.from, dur: l.dur })) }; });
        console.log(`C1 ${mode}: project layers loaded`);
        const extensions = await facts(app.page, ids);
        const ui = results[mode].ui;
        const after = await facts(app.page, ids);
        results[mode] = { ...results[mode], extensions, ui, after, setup, errors: app.diagnostics.pageErrors, frames: [], openDone: await app.page.evaluate(() => (window as any).__c1OpenDone) };
        console.log(`C1 ${mode}: UI exercised; capturing corpus frames`);
        // Same real-project corpus schedule: each 2 s plus last frame.
        // Add midpoints inside each typewriter interval so it cannot pass
        // by checking only times when the custom text is absent.
        const times = [...new Set([0, ...Array.from({ length: Math.ceil(setup.dur / 2) }, (_, i) => i * 2), Math.max(0, setup.dur - 1 / setup.fps), ...setup.typewriters.map((l: any) => Math.min(setup.dur - 1 / setup.fps, l.from + l.dur / 2))])].sort((a, b) => a - b);
        for (let i = 0; i < times.length; i++) {
          const time = times[i]!;
          results[mode].pendingFrame = time;
          await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
          const rendered = await frame(app.page, time, setup.w, setup.h);
          const bytes = Buffer.from(rendered.rgba, 'base64');
          const png = path.join(EVIDENCE, `${mode}-${i}.png`);
          await writeFile(png, Buffer.from(rendered.png.split(',')[1]!, 'base64'));
          if (!engine) {
            const baseline = path.join(EVIDENCE, `today-${i}.rgba`); await writeFile(baseline, bytes);
            captures.push({ time, baseline, png, width: rendered.width, height: rendered.height });
            results[mode].frames.push({ time, png, width: rendered.width, height: rendered.height });
          } else {
            const reference = captures[i]!;
            results[mode].frames.push({ time, png, reference: reference.png, ...measure(await readFile(reference.baseline), bytes) });
          }
          delete results[mode].pendingFrame;
          await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
        }
        results[mode].status = await app.page.evaluate(() => (window as any).PM.EngineConnection?.status?.() ?? null);

        results[mode].uiAfter = await exercise(app.page, ids);
        results[mode].after = await facts(app.page, ids);
        results[mode].openDone = await app.page.evaluate(() => (window as any).__c1OpenDone);

      } catch (error) {
        results[mode] ??= {}; results[mode].failure = String(error);
        results[mode].extensions = await facts(app.page, ids).catch(() => results[mode].extensions ?? []);
        results[mode].failureState = await app.page.evaluate(() => {
          const PM = (window as any).PM;
          return { name: PM?.proj?.name, layers: PM?.proj?.layers?.length, toasts: (window as any).__c1Toasts ?? [], openDone: (window as any).__c1OpenDone, frame: (window as any).__c1Frame, status: PM?.EngineConnection?.status?.() ?? null };
        }).catch(() => null);
      }
      finally {
        // The disposable real project has a large saved history. Do not let
        // a quit/save decision hold the shared Electron lock indefinitely.
        app.app.process().kill('SIGKILL');
        await app.close();
      }
      await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
    }
    const comparison = ids.map((id) => { const today = results.today?.extensions?.find((r: any) => r.id === id); const engine = results.engine?.extensions?.find((r: any) => r.id === id); return { id, today, engine, same: JSON.stringify(today) === JSON.stringify(engine) }; });
    results.comparison = comparison;
    await writeFile(path.join(EVIDENCE, 'results.json'), JSON.stringify(results, null, 2));
    console.log(`C1 private per-extension results: ${path.join(EVIDENCE, 'results.json')}`);
    expect(results.today?.failure).toBeUndefined(); expect(results.engine?.failure).toBeUndefined();
    expect(comparison.filter((r) => !r.same)).toEqual([]);
    // The corpus deliberately excludes private-extension-32's runtime dependency:
    // preserve/report its same compile failure, never mask any other one.
    for (const mode of ['today', 'engine']) {
      expect(results[mode].openWaitError, `${mode}: project-open deadline`).toBeUndefined();
      expect(results[mode].openCompletionError, `${mode}: project-open completion deadline`).toBeUndefined();
      expect(results[mode].openDone, `${mode}: original project open completed`).toBe(true);
      for (const records of [results[mode].extensions, results[mode].after]) expect(records.filter((r: any) => r.id !== 'private-extension-32' && (!r.active || !r.enabled || r.health !== 'ok'))).toEqual([]);
      for (const ui of [results[mode].ui, results[mode].uiAfter]) {
        expect(ui.panels.filter((p: any) => !p.mounted || p.error)).toEqual([]);
        expect(ui.commands.filter((c: any) => !c.ok)).toEqual([]);
        expect(ui.theme.tools).toBeGreaterThanOrEqual(8);
        expect(ui.theme.stage && ui.theme.timeline).toBe(true);
      }
    }
    expect(results.engine.ui).toEqual(results.today.ui);
    expect(results.engine.uiAfter).toEqual(results.today.uiAfter);
    expect(results.engine.ui.gradient).toBe('private-extension-15');
    expect(results.engine.ui.rasterWrapped && results.engine.ui.legacyPm).toBe(true);
    expect(results.engine.frames.length).toBe(captures.length);
    expect(results.engine.frames.filter((f: any) => !f.pass)).toEqual([]);
    expect(results.engine.wrappers.surfaces).toEqual(results.today.wrappers.surfaces);
    expect(results.engine.wrappers.surfaces.every((s: any) => s.sync && s.wrapper)).toBe(true);
    expect(results.engine.wrappers.frames.filter((f: any) => !f.pass)).toEqual([]);
  } finally {
    for (const profile of profiles) await rm(profile, { recursive: true, force: true });
    for (const capture of captures) await rm(capture.baseline, { force: true });
    for (const time of [.2, 1.2]) await rm(path.join(EVIDENCE, `today-wrappers-${time}.rgba`), { force: true });
  }
});
