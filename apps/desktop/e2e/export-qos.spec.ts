import { mkdtemp, rm, writeFile, mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { launchApp, type LaunchedApp } from './helpers/app';

/* EXPORTQOS-1: a user job begins while idle, playback starts during it,
   stops, and the job continues. Thread-class transitions are also checked
   against Darwin in pm-node/jobs/qos.rs. No legacy rendering is invoked. */
test('user export protects playback cadence across playback start and stop', async ({}, info) => {
  test.setTimeout(180_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-qos-'));
  let session: LaunchedApp | undefined;
  const metrics: Record<string, unknown> = { load: os.loadavg() };
  try {
    session = await launchApp({ userData: path.join(root, 'profile'), env: { POWERMOVE_NEW_ENGINE: '1' } });
    await session.openEditor();
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.proj = PM.mkProject({ name: 'Export QoS', w: 1920, h: 1080, fps: 60, dur: 120, bg: '#132641' });
      for (let i = 0; i < 120; i++) {
        const layer = PM.mkLayer('solid', { name: `Tile ${i}`, from: 0, dur: 120,
          d: { color: i % 2 ? '#e83a5e' : '#4ad1c8', w: 90, h: 90 } });
        layer.p['position.x'].v = (i % 15) * 120 - 840;
        layer.p['position.y'].v = Math.floor(i / 15) * 120 - 420;
        layer.p['rotation'].kf = [PM.KF(0, 0), PM.KF(120, 360)];
        PM.proj.layers.push(layer);
      }
      PM.ProjectIndex.invalidate(); PM.touch();
      PM.bus.emit('project'); PM.bus.emit('layers');
    });
    await expect.poll(() => session!.page.evaluate(() => !!(window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.latest), { timeout: 45_000 }).toBe(true);
    const runtimeQos = () => session!.app.evaluate(async () =>
      (await (globalThis as any).__powermoveNativeEngine?.engine?.report('job'))?.qos ?? null);
    const sample = async (exporting = false) => {
      const sampled = session!.page.evaluate(async () => {
        const PM = (window as any).PM;
        const presentation = PM.Kernel.services.get('viewer').enginePresentation;
        PM.loop = false; PM.setTime(0); PM.play();
        await new Promise(resolve => setTimeout(resolve, 400));
        const stamps: number[] = [];
        let seq = -1;
        const off = presentation.onPresented((frame: any) => {
          if (frame.seq !== seq) { seq = frame.seq; stamps.push(performance.now()); }
        });
        try { await new Promise(resolve => setTimeout(resolve, 3000)); }
        finally { off(); PM.pause(); }
        const intervals = stamps.slice(1).map((t, i) => t - stamps[i]!).sort((a, b) => a - b);
        return { frames: stamps.length, p50: intervals[Math.floor(intervals.length * 0.5)] ?? null,
          p99: intervals[Math.floor(intervals.length * 0.99)] ?? null, max: intervals.at(-1) ?? null,
          late: intervals.filter(ms => ms > 2 * 1000 / 60).length };
      });
      const qos = (async () => {
        if (exporting) {
          await expect.poll(runtimeQos, { timeout: 10_000 }).toBe('utility');
          metrics.qosPlaying = await runtimeQos();
        }
      })();
      const [frames, checked] = await Promise.allSettled([sampled, qos]);
      if (frames.status === 'rejected') throw frames.reason;
      if (checked.status === 'rejected') throw checked.reason;
      return frames.value;
    };
    metrics.control = await sample();
    await session.app.evaluate(({ dialog }, file) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: file }); }, path.join(root, 'export.webm'));
    const start = await session.page.evaluate(async () => {
      const w = window as any, PM = w.PM, engine = w.__powermoveTestBridge.nativeEngine;
      w.__exportQosStatuses = [];
      engine.onExportStatus((status: any) => w.__exportQosStatuses.push(status));
      await PM.HostMedia?.flushBindings(PM.proj); await PM.EngineDefinitions?.settle?.();
      return engine.exportVideo({ project: JSON.stringify(PM.proj), name: 'Export QoS', options: {
        format: 'webm-vp9', alpha: false, fps: 60, scale: 1, start: 0, end: 120,
        audio: false, motionBlur: false, quality: 'high'
      } });
    });
    expect(start.status).toBe('accepted');
    await expect.poll(() => session!.page.evaluate(() => (window as any).__exportQosStatuses.some((s: any) => s.progress > 0)), { timeout: 45_000 }).toBe(true);
    await expect.poll(runtimeQos).toBe('default');
    metrics.qosIdle = await runtimeQos();
    metrics.during = await sample(true);
    await expect.poll(runtimeQos).toBe('default');
    metrics.qosStopped = await runtimeQos();
    const before = await session.page.evaluate(() => (window as any).__exportQosStatuses.at(-1));
    expect(before.state).toBe('running');
    await expect.poll(() => session!.page.evaluate(() => (window as any).__exportQosStatuses.at(-1)?.progress), { timeout: 30_000 }).toBeGreaterThan(before.progress);
    await session.page.evaluate(async (id) => { await (window as any).__powermoveTestBridge.nativeEngine.cancelExport(id); }, start.jobId);
    try { await expect.poll(() => session!.page.evaluate(() => (window as any).__exportQosStatuses.at(-1)?.state)).toBe('cancelled'); }
    finally { metrics.statuses = await session.page.evaluate(() => (window as any).__exportQosStatuses.map(({ state, progress, routed, error }: any) => ({ state, progress, routed, error }))); }
    await expect.poll(runtimeQos).toBe('utility');
    metrics.qosCancelled = await runtimeQos();
    expect((metrics.during as any).frames).toBeGreaterThan(100);
    expect((metrics.during as any).late, 'zero presentation gaps beyond the two-frame deadline').toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    try { await session?.close(); }
    finally {
      try {
        await mkdir(info.outputDir, { recursive: true });
        await writeFile(path.join(info.outputDir, 'cadence.json'), JSON.stringify(metrics, null, 2));
      } finally { await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); }
    }
  }
});
