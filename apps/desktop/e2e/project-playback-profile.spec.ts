import { copyFile, mkdtemp, writeFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from './helpers/app';

test('profiles actual playback of a copied media project', async ({ session }) => {
  test.skip(!process.env.PM_PLAYBACK_PROJECT, 'Set PM_PLAYBACK_PROJECT to a project to measure');
  test.setTimeout(180_000);
  const directory = await mkdtemp(join(tmpdir(), 'powermove-playback-profile-'));
  const copy = join(directory, 'Project.pmv');
  await copyFile(process.env.PM_PLAYBACK_PROJECT!, copy);
  await session.openEditor();
  const viewport = { width: Number(process.env.PM_PLAYBACK_WIDTH) || 1728, height: Number(process.env.PM_PLAYBACK_HEIGHT) || 1117 };
  await session.page.setViewportSize(viewport);
  const cdp = await session.page.context().newCDPSession(session.page);
  if (process.env.PM_PLAYBACK_DPR) await cdp.send('Emulation.setDeviceMetricsOverride', {
    ...viewport, deviceScaleFactor: Number(process.env.PM_PLAYBACK_DPR), mobile: false,
  });
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, copy);
  await session.page.evaluate(() => (window as any).PM.openProject());
  await session.page.waitForFunction(() => !(window as any).PM.assets.loading?.size);
  // A tool snapshot can contain newer artwork than the saved file. Restore
  // the file's media first, then inspect the current composition with it.
  if (process.env.PM_PLAYBACK_SNAPSHOT) {
    const snapshot = JSON.parse(readFileSync(process.env.PM_PLAYBACK_SNAPSHOT, 'utf8'));
    await session.page.evaluate(data => {
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data }));
    }, snapshot);
    await session.page.waitForFunction(() => !(window as any).PM.assets.loading?.size);
  }
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    if (PM.Comps) {
      const largest: any = PM.Comps.list().sort((a: any, b: any) => (PM.Comps.get(b.id)?.layers.length || 0) - (PM.Comps.get(a.id)?.layers.length || 0))[0];
      if (largest) PM.Comps.open(largest.id);
    } else {
      // Profile nested footage on builds without composition-tab navigation too.
      const project = PM.proj;
      const largest: any = Object.values(project.comps || {}).sort((a: any, b: any) => b.layers.length - a.layers.length)[0];
      if (largest?.layers.length > project.layers.length) PM.replaceProject({ ...project, ...largest,
        id: project.id, name: project.name, assets: project.assets, comps: project.comps, work: [0, largest.dur] });
    }
    PM.pause(); PM.setTime(0); PM.loop = true;
  });
  await session.page.waitForTimeout(1000);
  const profileOrigin = await session.page.evaluate(() => performance.now());
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const results = await session.page.evaluate(async ({ seconds, auto, zooms, profileOrigin }) => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    const results = [], render = PM.GL.render, raster = PM.raster;
    const gl = PM.GL.gl, upload = gl.texImage2D, subUpload = gl.texSubImage2D;
    const storeGet = PM.store.get, storeSet = PM.store.set;
    let frames: any[] = [], costs = new Map<string, { calls: number; ms: number }>();
    let uploads = 0, allocations = 0, storeOps: any[] = [], longTasks: any[] = [], sources: any[] = [];
    gl.texImage2D = function (...args: any[]) {
      if (args.at(-1) == null) allocations++; else uploads++;
      return upload.apply(this, args);
    };
    gl.texSubImage2D = function (...args: any[]) { uploads++; return subUpload.apply(this, args); };
    const timedStore = (operation: string, original: any) => function (this: any, ...args: any[]) {
      const at = performance.now(), value = original.apply(this, args), ms = performance.now() - at;
      if (PM.playing && ms > 5) storeOps.push({ operation, key: String(args[0]), at, ms });
      return value;
    };
    PM.store.get = timedStore('get', storeGet); PM.store.set = timedStore('set', storeSet);
    const observer = new PerformanceObserver(list => {
      for (const entry of list.getEntries()) longTasks.push({ at: entry.startTime, ms: entry.duration });
    });
    observer.observe({ type: 'longtask' });
    PM.raster = (layer: any, ...args: any[]) => {
      const start = performance.now(), result = raster(layer, ...args);
      const ms = performance.now() - start;
      if (ms > 1 || args[1] > PM.time) sources.push({ id: layer.id, type: layer.type, time: args[1], currentTime: PM.time, scale: args[0], ms, keyLength: result?.key?.length });
      const item = costs.get(layer.type) || { calls: 0, ms: 0 }; item.calls++; item.ms += ms; costs.set(layer.type, item);
      return result;
    };
    PM.GL.render = (...args: any[]) => {
      const start = performance.now(), beforeUploads = uploads, beforeAllocations = allocations, result = render(...args);
      if (PM.playing) frames.push({ at: start, time: args[0], ms: performance.now() - start, presented: result !== false, quality: PM.quality,
        draws: PM.GL.stats.draws, passes: PM.GL.stats.passes, hit: PM.GL.stats.previewHit,
        uploads: uploads - beforeUploads, allocations: allocations - beforeAllocations });
      return result;
    };
    try {
      for (const zoom of zooms) {
        PM.pause(); PM.setTime(0); PM.perf.auto = auto; PM.quality = 1;
        viewer.fit = zoom === 0; viewer.zoom = zoom || 1; viewer.pan = [0, 0]; viewer.layout();
        await new Promise(resolve => setTimeout(resolve, 300));
        frames = []; costs = new Map(); storeOps = []; longTasks = []; sources = [];
        const start = performance.now(); PM.play();
        if (!PM.playing) throw new Error('Playback did not start; inspect project media restoration.');
        await new Promise(resolve => setTimeout(resolve, seconds * 1000));
        const elapsed = performance.now() - start; PM.pause();
        results.push({ zoom, elapsed, profileOrigin, frames, storeOps, longTasks, sources, raster: Object.fromEntries(costs), layers: PM.proj.layers.length,
          fps: PM.proj.fps, canvas: [PM.GL.canvas.width, PM.GL.canvas.height], memory: PM.Memory.stats() });
      }
    } finally {
      PM.pause(); PM.GL.render = render; PM.raster = raster;
      gl.texImage2D = upload; gl.texSubImage2D = subUpload;
      PM.store.get = storeGet; PM.store.set = storeSet; observer.disconnect();
    }
    return results;
  }, { seconds: Number(process.env.PM_PLAYBACK_SECONDS) || 21, auto: process.env.PM_PLAYBACK_AUTO === '1', zooms: (process.env.PM_PLAYBACK_ZOOMS || '0,1').split(',').map(Number), profileOrigin });
  const output = process.env.PM_PLAYBACK_OUTPUT || join(directory, 'playback.json');
  await writeFile(output, JSON.stringify(results));
  await writeFile(output + '.cpuprofile', JSON.stringify((await cdp.send('Profiler.stop')).profile));
  for (const result of results) {
    const costs = result.frames.map((frame: any) => frame.ms).sort((a: number, b: number) => a - b);
    const presented = result.frames.filter((frame: any) => frame.presented);
    const gaps = presented.slice(1).map((frame: any, index: number) => ({ time: frame.time,
      ms: frame.at - presented[index].at, previous: presented[index] })).sort((a: any, b: any) => b.ms - a.ms);
    console.log('PROJECT_PLAYBACK', JSON.stringify({ zoom: result.zoom, layers: result.layers, actualFps: result.frames.filter((f: any) => f.presented).length * 1000 / result.elapsed,
      median: costs[Math.floor(costs.length / 2)], p95: costs[Math.floor(costs.length * .95)], max: costs.at(-1),
      worstGaps: gaps.slice(0, 8), storeOps: result.storeOps, raster: result.raster, canvas: result.canvas }));
    expect(result.frames.length).toBeGreaterThan(10);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
