/**
 * Phase 4a app measurements. Prepare the 4K sources outside the Electron
 * lease, then run this file under pm-electron-lock. The test deliberately has
 * no opt-in skip: missing media is a failed measurement, not a passing row.
 *
 * PM_PHASE4A_PERF_MEDIA must contain 6-second (or longer) 3840x2160 sources:
 * hevc10-longgop-4k.mov, h26442210-longgop-4k.mov, prores-4k.mov.
 */
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import os from 'node:os';
import path from 'node:path';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const MEDIA = process.env.PM_PHASE4A_PERF_MEDIA;
const SOURCES = [
  { name: 'hevc10', file: 'hevc10-longgop-4k.mov' },
  { name: 'h26442210', file: 'h26442210-longgop-4k.mov' },
] as const;
const PRORES = 'prores-4k.mov';
const execFileAsync = promisify(execFile);

function source(file: string): string {
  expect(MEDIA, 'Set PM_PHASE4A_PERF_MEDIA to the prepared 4K fixture directory').toBeTruthy();
  const filename = path.join(MEDIA!, file);
  expect(existsSync(filename), `Missing prepared Phase 4a source: ${filename}`).toBe(true);
  return filename;
}

async function importClip(session: LaunchedApp, file: string): Promise<void> {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: '4K automatic preview', w: 3840, h: 2160,
      fps: 24, dur: 6, bg: '#000000' });
    PM.replaceProject(project);
    PM.pause(); PM.setTime(0);
    const input = document.createElement('input');
    input.id = 'phase4a-perf-input'; input.type = 'file'; document.body.appendChild(input);
  });
  await session.page.locator('#phase4a-perf-input').setInputFiles(file);
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.importFiles([(document.querySelector('#phase4a-perf-input') as HTMLInputElement).files![0]]);
    const layer = PM.proj.layers.find((item: any) => item.type === 'video');
    if (!layer) throw new Error('The imported source did not create a video layer');
    layer.from = 0; layer.dur = 6;
    PM.proj.work = [0, 6];
    PM.loop = true; PM.previewResolution = 'auto'; PM.quality = 1; PM.perf.auto = true;
    PM.Kernel.services.get('viewer').setMediaPreference?.('automatic');
    PM.touch(); PM.bus.emit('layers'); PM.bus.emit('quality');
    PM.setTime(.25, { raw: true, force: true });
  });
  await session.page.waitForFunction(() => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((item: any) => item.type === 'video');
    return !!layer && !!PM.assets.get(layer.d.asset)?.preview?.el;
  }, undefined, { timeout: 180_000 });
}

async function snapshot(session: LaunchedApp, native: boolean) {
  const page = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const state = PM.EngineConnection?.status?.();
    const layer = PM.proj.layers.find((item: any) => item.type === 'video');
    return { time: PM.time, playing: PM.playing, legacyFps: PM.perf?.fps ?? null,
      quality: PM.quality, route: state?.route ?? 'legacy',
      renderStats: state?.renderStats ?? null,
      frame: PM.EngineConnection?.session?.()?.lastFrame?.quality ?? null,
      proxyState: layer && PM.Proxies?.assetState?.(layer.d.asset) || null,
      errors: PM.EngineDoc?.failures?.() ?? [] };
  });
  const metrics = await session.app.evaluate(({ app }) => app.getAppMetrics().map(row => ({
    type: row.type, pid: row.pid, workingSetKB: row.memory.workingSetSize,
  })));
  const report = native ? await session.app.evaluate(() =>
    (globalThis as any).__powermoveNativeEngine.engine.report('interactive')) : null;
  const processes = native ? await session.app.evaluate(() =>
    (globalThis as any).__powermoveNativeEngine.engine.status()) : null;
  const pids = [...new Set([...metrics.map(row => row.pid), ...Object.values(processes ?? {})
    .map(row => (row as { pid: number | null }).pid).filter((pid): pid is number => Number.isInteger(pid) && pid! > 0)])];
  const { stdout } = await execFileAsync('/bin/ps', ['-o', 'pid=,rss=', '-p', pids.join(',')]);
  const rss = stdout.trim().split('\n').map(line => {
    const [pid, rssKB] = line.trim().split(/\s+/).map(Number);
    return { pid: pid!, rssKB: rssKB! };
  });
  expect(rss.length).toBeGreaterThan(0);
  for (const row of Object.values(processes ?? {}) as Array<{ pid: number | null }>)
    if (row.pid) expect(rss.some(sample => sample.pid === row.pid), 'Engine process RSS is missing').toBe(true);
  expect(rss.every(row => Number.isFinite(row.rssKB) && row.rssKB > 0)).toBe(true);
  return { ...page, metrics, processes, rss, report };
}

/** Measure settled Automatic playback after the real machine assessment and
 * any required background generation, without manually requesting a proxy. */
async function settleAutomatic(session: LaunchedApp): Promise<boolean> {
  const asset = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.Proxies.flush();
    const id = PM.proj.layers.find((layer: any) => layer.type === 'video').d.asset;
    const meta = PM.proj.assets[id];
    const record = PM.HostMedia.store.recordFor(meta.storageKey ?? id);
    return { id, hash: meta.contentHash ?? meta.link?.hash ?? record?.hash, project: PM.proj.id };
  });
  expect(asset.hash).toMatch(/^[a-f0-9]{64}$/);
  let needsProxy = false;
  await expect.poll(async () => {
    try {
      const assessed = JSON.parse(await readFile(path.join(session.userData, 'Engine Cache', 'Proxy Assessments.json'), 'utf8'));
      const row = assessed.results?.[asset.hash];
      if (typeof row?.needsProxy !== 'boolean') return false;
      needsProxy = row.needsProxy;
      return true;
    } catch { return false; }
  }, { timeout: 180_000, message: 'Automatic source assessment did not settle while paused' }).toBe(true);
  if (needsProxy) {
    await expect.poll(async () => {
      const snapshot = await session.page.evaluate(async ({ project }) => {
        const PM = (window as any).PM;
        return PM.EngineProxies.snapshot(project);
      }, asset);
      const state = snapshot.assets[asset.id];
      if (snapshot.activity.paused === 'lowDisk')
        throw new Error(`Automatic generation paused by disk guard: ${JSON.stringify({ state, activity: snapshot.activity })}`);
      if (state?.state === 'failed') throw new Error(state.error);
      return state?.state;
    }, { timeout: 180_000, message: 'Automatic generation did not finish while paused' }).toBe('ready');
  }
  return needsProxy;
}

async function playFor(session: LaunchedApp, native: boolean, durationMs: number, intervalMs = 5_000) {
  const initial = await snapshot(session, native);
  await session.page.evaluate((usingNative) => {
    const PM = (window as any).PM;
    const counter = { count: 0, firstMs: null as number | null, lastMs: null as number | null,
      representations: {} as Record<string, number>,
      stop: () => {} };
    const count = () => { const now = performance.now();
      counter.count++; counter.firstMs ??= now; counter.lastMs = now; };
    if (usingNative) {
      const presentation = PM.Kernel.services.get('viewer').enginePresentation;
      let previous = Number.NaN;
      const off = presentation.onPresented((frame: any) => {
        if (frame.timed && !frame.standIn && frame.pts !== previous) {
          previous = frame.pts;
          const representation = frame.quality?.representation ?? 'missing';
          counter.representations[representation] = (counter.representations[representation] ?? 0) + 1;
          count();
        }
      });
      counter.stop = off;
    } else {
      let previous = Number.NaN;
      counter.stop = PM.bus.on('preview:presented', (presented: { time: number }) => {
        const frame = Math.round(Number(presented.time) * 24);
        if (Number.isFinite(frame) && frame !== previous) {
          previous = frame; count();
        }
      });
    }
    (window as any).__phase4aPerf = counter;
    PM.setTime(.25, { raw: true, force: true }); PM.play();
  }, native);
  const samples = [];
  const began = Date.now();
  try {
    while (Date.now() - began < durationMs) {
      await new Promise(resolve => setTimeout(resolve, Math.min(intervalMs, Math.max(0, durationMs - (Date.now() - began)))));
      samples.push({ elapsedMs: Date.now() - began, ...(await snapshot(session, native)) });
    }
  } finally {
    await session.page.evaluate(() => (window as any).PM.pause()).catch(() => undefined);
  }
  const elapsedMs = Date.now() - began;
  const presented = await session.page.evaluate(() => {
    const state = (window as any).__phase4aPerf;
    state.stop(); delete (window as any).__phase4aPerf;
    return { count: state.count, firstMs: state.firstMs, lastMs: state.lastMs, representations: state.representations };
  });
  return { initial, samples, final: await snapshot(session, native), elapsedMs, presented,
    presentedFps: presented.count * 1_000 / elapsedMs };
}

async function pauseFirstFrame(session: LaunchedApp, playingPreference: 'automatic' | 'proxies') {
  return session.page.evaluate(async (preference) => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const presentation = viewer.enginePresentation;
    viewer.setMediaPreference(preference);
    PM.setTime(.25, { raw: true, force: true }); PM.play();
    await new Promise(resolve => setTimeout(resolve, 2_000));
    const playing = PM.EngineConnection.session()?.lastFrame?.quality?.representation ?? null;
    const before = performance.now();
    const frames: Array<{ ms: number; representation: string | null; standIn: boolean }> = [];
    const first = await new Promise<{ ms: number; representation: string | null; standIn: boolean } | null>(resolve => {
      let done = false;
      const off = presentation.onPresented((frame: any) => {
        if (done || PM.playing) return;
        const row = { ms: frame.rafMs - before,
          representation: frame.quality?.representation ?? null,
          standIn: !!frame.standIn };
        frames.push(row);
        if (row.standIn) return;
        done = true; off(); resolve(row);
      });
      PM.pause();
      if (preference === 'proxies') viewer.setMediaPreference('automatic');
      PM.invalidate('render');
      setTimeout(() => { if (!done) { done = true; off(); resolve(null); } }, 3_000);
    });
    return { playing, first, frames, final: PM.EngineConnection.session()?.lastFrame?.quality ?? null };
  }, playingPreference);
}

test('paired 60-second Automatic 4K long-GOP playback uses measured presentation counters', async ({}, testInfo) => {
  test.setTimeout(900_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-phase4a-paired-'));
  const rows = [];
  try {
    for (const clip of SOURCES) {
      const filename = source(clip.file);
      const pair = [];
      for (const native of [false, true]) {
        let session: LaunchedApp | undefined;
        try {
          session = await launchApp({ userData: path.join(root, `${clip.name}-${native ? 'native' : 'current'}`),
            env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' } });
          await importClip(session, filename);
          if (native) await session.page.waitForFunction(() =>
            (window as any).PM.EngineConnection?.status?.().route === 'engine');
          const needsProxy = native ? await settleAutomatic(session) : null;
          const result = await playFor(session, native, 60_000);
          pair.push(result);
          rows.push({ source: clip.name, engine: native ? 'native' : 'current', needsProxy, ...result });
          expect(result.samples.length).toBeGreaterThanOrEqual(11);
          expect(result.samples.some(sample => sample.playing)).toBe(true);
          if (native) {
            expect(result.samples.some(sample => sample.route === 'engine')).toBe(true);
            const first = result.initial.renderStats;
            const last = result.final.renderStats;
            expect(typeof first?.timedFrames).toBe('number');
            expect(typeof last?.timedFrames).toBe('number');
            expect(last!.timedFrames).toBeGreaterThan(first!.timedFrames);
            expect(typeof first?.lateSkipped).toBe('number');
            expect(typeof last?.lateSkipped).toBe('number');
            expect(last!.lateSkipped - first!.lateSkipped).toBe(0);
            expect(result.presentedFps).toBeGreaterThanOrEqual(23.5);
            expect(result.presented.representations.missing ?? 0).toBe(0);
            expect(result.samples.at(-1)?.errors).toEqual([]);
          } else {
            expect(result.samples.some(sample => (sample.legacyFps ?? 0) > 0)).toBe(true);
          }
        } finally { await session?.close(); }
      }
      expect(pair[1]!.presentedFps, `${clip.name}: native presented fps fell below current`).toBeGreaterThanOrEqual(pair[0]!.presentedFps - .5);
    }
  } finally {
    await testInfo.attach('phase4a-paired-playback.json', { body: Buffer.from(JSON.stringify(rows)), contentType: 'application/json' });
    await rm(root, { recursive: true, force: true });
  }
});

test('paused ProRes and HEVC upgrade to original on the first presented frame', async ({}, testInfo) => {
  test.setTimeout(600_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-phase4a-upgrade-'));
  const rows = [];
  try {
    for (const file of [PRORES, SOURCES[0].file]) {
      let session: LaunchedApp | undefined;
      try {
        session = await launchApp({ userData: path.join(root, file), env: { POWERMOVE_NEW_ENGINE: '1' } });
        await importClip(session, source(file));
        await session.page.waitForFunction(() => (window as any).PM.EngineConnection?.status?.().route === 'engine');
        await settleAutomatic(session);
        const automatic = await pauseFirstFrame(session, 'automatic');
        const row: { source: string; automatic: typeof automatic; fromProxy?: typeof automatic } = { source: file, automatic };
        rows.push(row);
        expect(automatic.first, `No Automatic paused frame presented for ${file}`).not.toBeNull();
        expect(automatic.first?.representation).toBe('original');
        expect(automatic.first?.ms).toBeGreaterThanOrEqual(0);
        expect(automatic.first?.ms).toBeLessThanOrEqual(file === PRORES ? 150 : 300);
        const asset = await session.page.evaluate(async (folder) => {
          const PM = (window as any).PM;
          const id = String(PM.proj.layers.find((layer: any) => layer.type === 'video').d.asset);
          PM.Proxies.setIntent({ create: 'onlyWhenAsked', size: 'half' });
          await PM.Proxies.flush();
          await PM.Proxies.setLocation({ kind: 'folder', path: folder });
          await PM.Proxies.create(id);
          return id;
        }, path.join(root, `${file}-proxies`));
        await session.page.waitForFunction((id) =>
          ['ready', 'failed'].includes((window as any).PM.Proxies.assetState(id).state),
        asset, { timeout: 180_000 });
        expect(await session.page.evaluate((id) => (window as any).PM.Proxies.assetState(id), asset))
          .toMatchObject({ state: 'ready' });
        const result = await pauseFirstFrame(session, 'proxies');
        row.fromProxy = result;
        expect(result.playing).toBe('proxy');
        expect(result.first, `No paused frame presented for ${file}`).not.toBeNull();
        expect(result.first?.representation).toBe('original');
        expect(result.first?.ms).toBeGreaterThanOrEqual(0);
        expect(result.first?.ms).toBeLessThanOrEqual(file === PRORES ? 150 : 300);
        expect(result.frames.filter(frame => !frame.standIn)).toHaveLength(1);
      } finally { await session?.close(); }
    }
  } finally {
    await testInfo.attach('phase4a-paused-upgrade.json', { body: Buffer.from(JSON.stringify(rows)), contentType: 'application/json' });
    await rm(root, { recursive: true, force: true });
  }
});

test('one-hour native playback keeps process memory and native cache bounded', async ({}, testInfo) => {
  test.setTimeout(3_800_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-phase4a-hour-'));
  let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ userData: root, env: { POWERMOVE_NEW_ENGINE: '1' } });
    await importClip(session, source(SOURCES[0].file));
    await settleAutomatic(session);
    const first = await playFor(session, true, 3_600_000, 60_000);
    const points = first.samples;
    const workingSet = (sample: typeof first.final) => sample.rss.reduce((sum, row) => sum + row.rssKB, 0);
    const warmup = points.slice(10, 16).map(workingSet);
    const tail = points.slice(-5).map(workingSet);
    const avg = (values: number[]) => values.reduce((sum, value) => sum + value, 0) / values.length;
    expect(points.length).toBeGreaterThanOrEqual(59);
    expect(tail.length).toBe(5);
    let previousFrames = first.initial.renderStats?.timedFrames ?? 0;
    for (const point of points) {
      expect(point.playing, 'Playback stopped during the memory run').toBe(true);
      expect(point.route).toBe('engine');
      expect(point.renderStats?.timedFrames, 'The native engine stopped producing timed frames').toBeGreaterThan(previousFrames);
      previousFrames = point.renderStats!.timedFrames;
    }
    // Chromium/VT caches can settle during the first 10 minutes. Thereafter
    // test both the endpoint plateau and a least-squares trend. Its ordinary
    // residual-based upper bound is a leak heuristic, not a confidence claim
    // for autocorrelated memory samples.
    const fit = points.slice(10).map(point => ({ x: point.elapsedMs / 60_000, y: workingSet(point) }));
    const meanX = avg(fit.map(point => point.x));
    const meanY = avg(fit.map(point => point.y));
    const sxx = fit.reduce((sum, point) => sum + (point.x - meanX) ** 2, 0);
    const slopeKBPerMinute = fit.reduce((sum, point) => sum + (point.x - meanX) * (point.y - meanY), 0) / sxx;
    const intercept = meanY - slopeKBPerMinute * meanX;
    const residual = fit.reduce((sum, point) => sum + (point.y - intercept - slopeKBPerMinute * point.x) ** 2, 0);
    const slopeUpper95KBPerMinute = slopeKBPerMinute + 1.96 * Math.sqrt(residual / (fit.length - 2) / sxx);
    await testInfo.attach('phase4a-one-hour-memory.json', { body: Buffer.from(JSON.stringify({ points, warmupKB: avg(warmup), tailKB: avg(tail), slopeKBPerMinute, slopeUpper95KBPerMinute })), contentType: 'application/json' });
    expect(avg(tail) - avg(warmup), 'Process working-set plateau rose by more than 128 MiB after warmup').toBeLessThan(128 * 1024);
    expect(slopeUpper95KBPerMinute, 'Post-warmup process memory has a sustained positive slope').toBeLessThan(512);
    for (const point of points) {
      expect(point.processes).toEqual(first.initial.processes);
      const report = point.report as any;
      for (const field of ['leaseBytes', 'usedBytes'])
        expect(Number.isFinite(report?.[field]), `Missing native ${field} telemetry`).toBe(true);
      expect(report.usedBytes).toBeLessThanOrEqual(report.leaseBytes);
      for (const field of ['mediaCacheBytes', 'mediaCacheBudget', 'previewCacheBytes', 'previewCacheBudget'])
        expect(Number.isFinite(report?.presenter?.[field]), `Missing native presenter ${field} telemetry`).toBe(true);
      expect(report.presenter.mediaCacheBudget).toBeGreaterThan(0);
      expect(report.presenter.mediaCacheBytes).toBeLessThanOrEqual(report.presenter.mediaCacheBudget);
      expect(report.presenter.previewCacheBytes).toBeLessThanOrEqual(report.presenter.previewCacheBudget);
    }
    expect((first.final.report as any)?.owedAck).toBeNull();
    expect(first.final.errors).toEqual([]);
  } finally { try { await session?.close(); } finally { await rm(root, { recursive: true, force: true }); } }
});
