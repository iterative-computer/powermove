import { mkdir, mkdtemp, rename, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test, type TestInfo } from '@playwright/test';

import { launchApp, type LaunchedApp } from './helpers/app';
import { compareCapture, compareExports, installLegacyWebmCapture, saveLegacyWebmCapture,
  type ParityReport } from './helpers/export-encoding-parity';
import { importFixture } from './helpers/media';

/* EXPORT-1 (export encoding parity): for the same settings, an export with
   Use New Engine on must hand today's encoders today's frames and samples,
   use today's encoders and settings, and decode to today's pictures and
   samples with today's stream properties. Each fixture exports every format
   twice with the setting off (today against itself: its own variance) and
   once with it on, then compares the encoder inputs (captured at the
   encoder boundary in hidden test launches), the decoded media, the stream
   properties, the encoded audio bytes, and the export time. */

type Format = { id: string; format: 'prores' | 'mp4' | 'webm'; alpha: boolean; extension: string };
type Mode = 'legacy' | 'native';
type Fixture = { name: 'basic' | 'advanced' | 'large'; width: number; height: number; fps: number; dur: number; capture: boolean };
type Exported = { file: string; captures: Record<string, string>; elapsedMs: number; statuses: Array<{ state: string; routed?: boolean; error?: string }> };

const FORMATS: Format[] = [
  { id: 'prores', format: 'prores', alpha: false, extension: '.mov' },
  { id: 'prores-alpha', format: 'prores', alpha: true, extension: '.mov' },
  { id: 'mp4', format: 'mp4', alpha: false, extension: '.mp4' },
  { id: 'webm', format: 'webm', alpha: false, extension: '.webm' },
];
const FIXTURES: Fixture[] = [
  { name: 'basic', width: 320, height: 180, fps: 30, dur: 1, capture: true },
  { name: 'advanced', width: 320, height: 180, fps: 30, dur: 1, capture: true },
  // Timing and multi-threaded encoders (libvpx uses 8 threads at 1080p);
  // no captures, which would slow both exporters.
  { name: 'large', width: 1920, height: 1080, fps: 30000 / 1001, dur: 2.002, capture: false },
];

const frames = (f: Fixture) => Math.round(f.dur * f.fps);
const captureNames = (mode: Mode, format: Format): string[] => {
  const prefix = mode === 'legacy' ? 'legacy' : 'native';
  return [`${prefix}.rgba`, format.format === 'webm' ? `${prefix}.f32` : `${prefix}.wav`];
};

async function engineReady(session: LaunchedApp): Promise<void> {
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
  await importFixture(session.page, 'tone.wav');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'audio'));
  await session.page.evaluate(({ advanced, dur }) => {
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
          speed: PM.P(1.2), fadeIn: PM.P(0.08), fadeOut: PM.P(0.1) } }, nested)];
      PM.proj.comps[nested.id] = nested;
      PM.proj.layers.push(PM.mkLayer('precomp', { name: 'Nested mix', from: 0, dur: 1,
        d: { comp: nested.id, audioGain: PM.P(0.75), trim: PM.P(0) } }, PM.proj));
    }
    PM.ProjectIndex.invalidate();
    PM.touch();
    PM.bus.emit('project'); PM.bus.emit('layers'); PM.bus.emit('assets');
  }, { advanced: fixture.name === 'advanced', dur: fixture.dur });
}

async function runExport(session: LaunchedApp, mode: Mode, format: Format, fixture: Fixture, serial: number,
  root: string, captureDir: string | null): Promise<Exported> {
  const basename = `${mode}-${format.id}-${serial}`;
  const file = path.join(root, basename + format.extension);
  const names = captureNames(mode, format);
  if (captureDir) await Promise.all(names.map(name => rm(path.join(captureDir, name), { force: true })));
  const webmCapture = !!captureDir && mode === 'legacy' && format.format === 'webm';
  if (webmCapture) await installLegacyWebmCapture(session.page);
  await session.app.evaluate(({ dialog }, output) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
  }, file);
  if (mode === 'native') await session.page.evaluate(() => { (window as any).__parityStatuses = []; });
  let webmErrors: string[] = [];
  let run: { result: { cancelled?: boolean; error?: string }; elapsedMs: number } | undefined;
  try {
    run = await session.page.evaluate(async ({ selected, alpha, fps }) => {
      const PM = (window as any).PM;
      const started = performance.now();
      const result = await PM.Export.run({ format: selected, scale: 1, fps,
        range: 'all', quality: 'high', mblur: false, alpha, audio: true });
      while (PM.Export.busy) await new Promise(resolve => setTimeout(resolve, 20));
      return { result, elapsedMs: performance.now() - started };
    }, { selected: format.format, alpha: format.alpha, fps: fixture.fps });
  } finally {
    if (webmCapture) webmErrors = await saveLegacyWebmCapture(session.page, captureDir!);
  }
  if (webmErrors.length) throw new Error(`WebCodecs input capture failed: ${webmErrors.join('; ')}`);
  if (run?.result?.cancelled || run?.result?.error) throw new Error(`${basename}: ${run.result.error ?? 'cancelled'}`);
  if (!(await stat(file).catch(() => null))) throw new Error(`${basename}: output was not written`);
  const captures: Record<string, string> = {};
  if (captureDir) {
    const archived = path.join(root, basename + '-inputs');
    await mkdir(archived);
    for (const name of names) {
      const source = path.join(captureDir, name), destination = path.join(archived, name);
      if (await stat(source).catch(() => null)) {
        await rm(destination, { force: true });
        await rename(source, destination);
        captures[name] = destination;
      }
    }
  }
  const statuses = mode === 'native'
    ? await session.page.evaluate(() => (window as any).__parityStatuses as Exported['statuses'])
    : [];
  return { file, captures, elapsedMs: run!.elapsedMs, statuses };
}

function softReport(label: string, report: ParityReport, all: Record<string, unknown>): void {
  all[label] = report.metrics;
  const failed = report.checks.filter(check => !check.equal);
  all[`${label}.failed`] = failed.map(check => ({ name: check.name, expected: check.expected, actual: check.actual }));
  for (const check of report.checks) {
    expect.soft(check.actual, `${label}: ${check.name} (expected ${JSON.stringify(check.expected)})`).toEqual(check.expected);
  }
}

async function comparePair(label: string, reference: Exported, candidate: Exported, format: Format, fixture: Fixture,
  evidence: Record<string, unknown>): Promise<void> {
  try {
    softReport(`${label}.media`, compareExports(reference.file, candidate.file, frames(fixture)), evidence);
  } catch (error) {
    evidence[`${label}.mediaError`] = String(error);
    expect.soft(String(error), `${label}: inspect encoded media`).toBe('inspected');
  }
  if (!fixture.capture) return;
  const rawName = format.format === 'webm' ? 'f32' : 'wav';
  const prefix = (e: Exported) => e.file.includes(`${path.sep}native-`) ? 'native' : 'legacy';
  const capture = (e: Exported, ext: string) =>
    e.captures[`${prefix(e)}.${ext}`] ?? path.join(path.dirname(e.file), `missing-${prefix(e)}.${ext}`);
  try {
    softReport(`${label}.preencode-rgba`, await compareCapture(capture(reference, 'rgba'), capture(candidate, 'rgba'), 'rgba',
      frames(fixture) * fixture.width * fixture.height * 4), evidence);
  } catch (error) {
    evidence[`${label}.rgbaError`] = String(error);
    expect.soft(String(error), `${label}: inspect preencode RGBA`).toBe('inspected');
  }
  try {
    softReport(`${label}.preencode-audio`, await compareCapture(capture(reference, rawName), capture(candidate, rawName), rawName), evidence);
  } catch (error) {
    evidence[`${label}.audioInputError`] = String(error);
    expect.soft(String(error), `${label}: inspect preencode audio`).toBe('inspected');
  }
}

for (const fixture of FIXTURES) {
  test(`export encoding parity: ${fixture.name} project`, async ({}, testInfo: TestInfo) => {
    test.setTimeout(1_800_000);
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-encoding-'));
    const captureDir = fixture.capture ? path.join(root, 'live-inputs') : null;
    const evidence: Record<string, unknown> = { fixture, formats: FORMATS.map(f => f.id), frames: frames(fixture) };
    let legacy: LaunchedApp | undefined, native: LaunchedApp | undefined;
    const old = new Map<string, Exported[]>();
    const current = new Map<string, Exported[]>();
    const env = (setting: string) => ({ POWERMOVE_NEW_ENGINE: setting, ...(captureDir ? { PM_EXPORT_INPUT_CAPTURE_DIR: captureDir } : {}) });
    try {
      if (captureDir) await mkdir(captureDir);
      legacy = await launchApp({ userData: path.join(root, 'legacy-profile'), env: env('0') });
      await project(legacy, fixture);
      for (const format of FORMATS) for (let serial = 0; serial < 2; serial++) {
        try {
          const exported = await runExport(legacy, 'legacy', format, fixture, serial, root, captureDir);
          old.set(format.id, [...(old.get(format.id) ?? []), exported]);
        } catch (error) {
          expect.soft(String(error), `legacy ${format.id} export ${serial}`).toBe('completed');
        }
      }
      await legacy.close(); legacy = undefined;

      native = await launchApp({ userData: path.join(root, 'native-profile'), env: env('1') });
      await engineReady(native);
      await project(native, fixture);
      await native.page.evaluate(() => {
        const w = window as any;
        w.__parityStatuses = [];
        w.__powermoveTestBridge.nativeEngine.onExportStatus((status: any) => {
          w.__parityStatuses.push({ state: status.state, routed: status.routed, error: status.error });
        });
      });
      // Timing: two engine exports when there are no captures.
      const nativeRuns = fixture.capture ? 1 : 2;
      for (const format of FORMATS) for (let serial = 0; serial < nativeRuns; serial++) {
        try {
          const exported = await runExport(native, 'native', format, fixture, serial, root, captureDir);
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
        if (pair.length < 2 || !candidates.length) continue;
        await comparePair(`${format.id}.today-repeat`, pair[0]!, pair[1]!, format, fixture, evidence);
        await comparePair(`${format.id}.engine`, pair[0]!, candidates[0]!, format, fixture, evidence);
        const times = { todayMs: pair.map(e => Math.round(e.elapsedMs)), engineMs: candidates.map(e => Math.round(e.elapsedMs)) };
        evidence[`${format.id}.time`] = times;
        if (!fixture.capture) {
          // Export time must not get slower than today: the engine's best
          // against today's best (both runs warm after the first).
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
