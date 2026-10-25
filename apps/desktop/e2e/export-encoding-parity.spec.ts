import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test, type TestInfo } from '@playwright/test';

import { launchApp, type LaunchedApp } from './helpers/app';
import { requestJobEngine } from './helpers/job-engine';
import { compareExports, type ParityReport } from './helpers/export-encoding-parity';
import { importFixture } from './helpers/media';

/* G-X1 (docs/engine/gate-policy.md; replaces EXPORT-1's byte identity with
   today's encoders, X-A2): for the same settings, an export with Use New
   Engine on keeps today's stream properties, and its decoded pictures are at
   least as close to the reference (the engine's lossless 16-bit PNG render of
   the same frames, exported alongside) as today's export. Each fixture exports
   every format with the setting off and on, and the large fixture twice with
   it on for the export time (pending a quiet machine for its verdict). */

type Format = { id: string; format: 'prores' | 'mp4' | 'webm'; alpha: boolean; extension: string };
type Mode = 'legacy' | 'native';
type Fixture = { name: 'basic' | 'automation' | 'advanced' | 'large'; width: number; height: number; fps: number; dur: number; capture: boolean };
type Exported = { file: string; elapsedMs: number; load?: { before: number[]; after: number[] }; statuses: Array<{ state: string; routed?: boolean; error?: string }> };

const FORMATS: Format[] = [
  { id: 'prores', format: 'prores', alpha: false, extension: '.mov' },
  { id: 'prores-alpha', format: 'prores', alpha: true, extension: '.mov' },
  { id: 'mp4', format: 'mp4', alpha: false, extension: '.mp4' },
  { id: 'webm', format: 'webm', alpha: false, extension: '.webm' },
];
const FIXTURES: Fixture[] = [
  { name: 'basic', width: 320, height: 180, fps: 30, dur: 1, capture: true },
  // Gain automation, fades, trims and a nested composition's levels.
  { name: 'automation', width: 320, height: 180, fps: 30, dur: 1, capture: true },
  // The same with the nested clip at speed 1.2.
  { name: 'advanced', width: 320, height: 180, fps: 30, dur: 1, capture: true },
  // Timing and multi-threaded encoders (libvpx uses 8 threads at 1080p);
  // no captures, which would slow both exporters.
  { name: 'large', width: 1920, height: 1080, fps: 30000 / 1001, dur: 2.002, capture: false },
];

const frames = (f: Fixture) => Math.round(f.dur * f.fps);

async function engineReady(session: LaunchedApp): Promise<void> {
  // The job utility starts on first demand (enginemem).
  await requestJobEngine(session.app);
  await expect.poll(() => session.app.evaluate(() => {
    const processes = (globalThis as any).__powermoveNativeEngine?.engine?.status();
    return processes ? `${processes.interactive?.state}/${processes.job?.state}` : 'missing';
  }), { timeout: 45_000 }).toBe('ready/ready');
}

async function project(session: LaunchedApp, fixture: Fixture): Promise<void> {
  await session.openEditor();
  await session.page.evaluate((f) => {
    const PM = (window as any).PM;
    PM.pause();
    PM.assets.clear();
    PM.proj = PM.mkProject({ name: 'Export encoding parity', w: f.width, h: f.height, fps: f.fps, dur: f.dur, bg: '#132641' });
    PM.proj.work = [0, f.dur];
    PM.time = 0;
    PM.sel.layers = [];
    PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Solid', from: 0, duration: f.dur,
      content: { color: '#e83a5e', w: f.width, h: f.height } }], { label: 'Parity solid', origin: 'e2e' });
    // Antialiased, rotated, translucent and moving: edges and blends.
    const shape = PM.mkLayer('shape', { name: 'Card', from: 0, dur: f.dur,
      d: { shape: 'rect', color: '#4AD1C8', w: f.width * 0.45, h: f.height * 0.5, radius: f.height * 0.08 } });
    shape.p['rotation'].v = 17;
    shape.p['opacity'].v = 62;
    shape.p['position.x'].v = f.width * 0.08;
    shape.p['position.x'].kf = [PM.KF(0, f.width * 0.08), PM.KF(f.dur, -f.width * 0.12)];
    PM.proj.layers.push(shape);
    PM.ProjectIndex.invalidate();
    PM.bus.emit('project');
    PM.bus.emit('layers');
  }, fixture);
  // PM_PARITY_FIXTURE=tone.m4a|tone.opus runs the same projects on a lossy
  // source (decoded by today's own FFmpeg in the engine, src/chromium.rs).
  await importFixture(session.page, process.env.PM_PARITY_FIXTURE ?? 'tone.wav');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'audio'));
  await session.page.evaluate(({ advanced, speed, dur }) => {
    const PM = (window as any).PM;
    const audio = PM.proj.layers.find((layer: any) => layer.type === 'audio');
    audio.from = 0;
    audio.dur = Math.min(dur, 1.5);
    audio.d.trim = PM.P(0);
    audio.d.gain = PM.P(0.8);
    audio.d.fadeIn = PM.P(0);
    audio.d.fadeOut = PM.P(0);
    if (advanced) {
      // Automation, fades, trims, a nested composition's level and speed.
      audio.from = 0.1;
      audio.dur = 0.85;
      audio.d.trim = PM.P(0.05);
      audio.d.gain = PM.P(0.25, { kf: [PM.KF(0, 0.25), PM.KF(0.4, 0.9), PM.KF(0.85, 0.4)] });
      audio.d.fadeIn = PM.P(0.14);
      audio.d.fadeOut = PM.P(0.2);
      const nested = PM.mkProject({ name: 'Nested audio', w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: 1 });
      nested.audioGain = PM.P(0.7);
      nested.layers = [PM.mkLayer('audio', { name: 'Nested tone', from: 0, dur: 1,
        d: { asset: audio.d.asset, gain: PM.P(0.35), trim: PM.P(0.1),
          speed: PM.P(speed), fadeIn: PM.P(0.08), fadeOut: PM.P(0.1) } }, nested)];
      PM.proj.comps[nested.id] = nested;
      PM.proj.layers.push(PM.mkLayer('precomp', { name: 'Nested mix', from: 0, dur: 1,
        d: { comp: nested.id, audioGain: PM.P(0.75), trim: PM.P(0) } }, PM.proj));
    }
    PM.ProjectIndex.invalidate();
    PM.touch();
    PM.bus.emit('project'); PM.bus.emit('layers'); PM.bus.emit('assets');
  }, { advanced: fixture.name === 'advanced' || fixture.name === 'automation', speed: fixture.name === 'advanced' ? 1.2 : 1, dur: fixture.dur });
}

async function runExport(session: LaunchedApp, mode: Mode, format: Format, fixture: Fixture, serial: number,
  root: string, reference = false): Promise<Exported> {
  const loadBefore = os.loadavg();
  const basename = `${mode}-${reference ? 'reference' : format.id}-${serial}`;
  const file = path.join(root, basename + (reference ? '' : format.extension));
  await session.app.evaluate(({ dialog }, output) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
  }, file);
  if (mode === 'native') await session.page.evaluate(() => { (window as any).__parityStatuses = []; });
  const run = await session.page.evaluate(async ({ selected, alpha, fps, reference }) => {
    const PM = (window as any).PM;
    const started = performance.now();
    const result = await PM.Export.run({ format: reference ? 'png' : selected, scale: 1, fps,
      range: 'all', quality: 'high', mblur: false, alpha, audio: !reference,
      ...(reference ? { delivery: { highDepth: true } } : {}) });
    while (PM.Export.busy) await new Promise(resolve => setTimeout(resolve, 20));
    return { result, elapsedMs: performance.now() - started };
  }, { selected: format.format, alpha: format.alpha, fps: fixture.fps, reference });
  if (run?.result?.cancelled || run?.result?.error) throw new Error(`${basename}: ${run.result.error ?? 'cancelled'}`);
  if (!(await stat(file).catch(() => null))) throw new Error(`${basename}: output was not written`);
  const statuses = mode === 'native'
    ? await session.page.evaluate(() => (window as any).__parityStatuses as Exported['statuses'])
    : [];
  return { file, elapsedMs: run!.elapsedMs, load: { before: loadBefore, after: os.loadavg() }, statuses };
}

function softReport(label: string, report: ParityReport, all: Record<string, unknown>): void {
  all[label] = report.metrics;
  const failed = report.checks.filter(check => !check.equal);
  all[`${label}.failed`] = failed.map(check => ({ name: check.name, expected: check.expected, actual: check.actual }));
  for (const check of report.checks) {
    expect.soft(check.actual, `${label}: ${check.name} (expected ${JSON.stringify(check.expected)})`).toEqual(check.expected);
  }
}

function comparePair(label: string, today: Exported, engine: Exported, reference: Exported | null, fixture: Fixture,
  evidence: Record<string, unknown>): void {
  try {
    softReport(label, compareExports(today.file, engine.file, reference?.file ?? null, frames(fixture), false), evidence);
  } catch (error) {
    evidence[`${label}.mediaError`] = String(error);
    expect.soft(String(error), `${label}: inspect encoded media`).toBe('inspected');
  }
}

for (const fixture of FIXTURES) {
  test(`export encoding parity: ${fixture.name} project`, async ({}, testInfo: TestInfo) => {
    test.setTimeout(1_800_000);
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-encoding-'));
    const evidence: Record<string, unknown> = { fixture, formats: FORMATS.map(f => f.id), frames: frames(fixture) };
    let legacy: LaunchedApp | undefined, native: LaunchedApp | undefined;
    const old = new Map<string, Exported[]>();
    const current = new Map<string, Exported[]>();
    const references = new Map<string, Exported>();
    const env = (setting: string) => ({ POWERMOVE_NEW_ENGINE: setting });
    try {
      legacy = await launchApp({ userData: path.join(root, 'legacy-profile'), env: env('0') });
      await project(legacy, fixture);
      for (const format of FORMATS) for (let serial = 0; serial < 1; serial++) {
        try {
          const exported = await runExport(legacy, 'legacy', format, fixture, serial, root);
          old.set(format.id, [...(old.get(format.id) ?? []), exported]);
        } catch (error) {
          expect.soft(String(error), `legacy ${format.id} export ${serial}`).toBe('completed');
        }
      }
      await legacy.close(); legacy = undefined;

      native = await launchApp({ userData: path.join(root, 'native-profile'), env: env('1') });
      if (process.env.PM_EXPORT_PROFILE === '1') {
        const log = (bytes: Buffer) => { const message = bytes.toString(); if (message.includes('[export timing]')) process.stdout.write(message); };
        native.app.process().stdout?.on('data', log); native.app.process().stderr?.on('data', log);
      }
      await engineReady(native);
      await project(native, fixture);
      await native.page.evaluate(() => {
        const w = window as any;
        w.__parityStatuses = [];
        w.__powermoveTestBridge.nativeEngine.onExportStatus((status: any) => {
          w.__parityStatuses.push({ state: status.state, routed: status.routed, error: status.error });
        });
      });
      // Timing: two engine exports of the large fixture.
      const nativeRuns = fixture.capture ? 1 : 2;
      // The reference: the engine's 16-bit PNG render of the same frames,
      // with and without alpha (small fixtures; the large one is timing).
      if (fixture.capture) for (const alpha of [false, true]) {
        try {
          references.set(String(alpha), await runExport(native, 'native', { ...FORMATS[0]!, alpha }, fixture, alpha ? 1 : 0, root, true));
        } catch (error) {
          expect.soft(String(error), `reference PNG (alpha ${alpha})`).toBe('completed');
        }
      }
      for (const format of FORMATS) for (let serial = 0; serial < nativeRuns; serial++) {
        try {
          const exported = await runExport(native, 'native', format, fixture, serial, root);
          current.set(format.id, [...(current.get(format.id) ?? []), exported]);
          expect.soft(exported.statuses.some(status => status.state === 'done'), `${format.id}: native completion status`).toBe(true);
          expect.soft(exported.statuses.some(status => status.routed), `${format.id}: no routed fallback`).toBe(false);
          expect.soft(exported.statuses.filter(status => status.state === 'failed'), `${format.id}: no failed native status`).toEqual([]);
        } catch (error) {
          expect.soft(String(error), `native ${format.id} export ${serial}`).toBe('completed');
        }
      }
      for (const format of FORMATS) {
        const pair = old.get(format.id) ?? [], candidates = current.get(format.id) ?? [];
        if (!pair.length || !candidates.length) continue;
        comparePair(`${format.id}.engine`, pair[0]!, candidates[0]!, references.get(String(format.alpha)) ?? null, fixture, evidence);
        const times = { todayMs: pair.map(e => Math.round(e.elapsedMs)), engineMs: candidates.map(e => Math.round(e.elapsedMs)), todayLoad: pair.map(e => e.load), engineLoad: candidates.map(e => e.load) };
        evidence[`${format.id}.time`] = times;
        if (!fixture.capture) {
          // Export time must not get slower than today (pending a quiet
          // machine for its verdict; the numbers are in metrics.json).
          expect.soft(Math.min(...times.engineMs), `${format.id}: engine export time vs today`).toBeLessThanOrEqual(Math.min(...times.todayMs));
        }
        await testInfo.attach(`today-${format.id}`, { path: pair[0]!.file });
        await testInfo.attach(`engine-${format.id}`, { path: candidates[0]!.file });
      }
    } finally {
      await Promise.allSettled([legacy?.close(), native?.close()]);
      try {
        await mkdir(testInfo.outputDir, { recursive: true });
        await writeFile(path.join(testInfo.outputDir, 'metrics.json'), JSON.stringify(evidence, null, 2));
      } finally {
        await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
      }
    }
  });
}
