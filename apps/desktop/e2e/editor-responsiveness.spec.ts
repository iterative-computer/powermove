import { readFileSync, writeFileSync, cpSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

test('random scrubbing stays responsive across a copied production composition', async ({ session }, info) => {
  test.skip(!process.env.PM_PERF_PROJECT && !process.env.PM_PERF_FILE, 'Provide a local project copy or saved file to profile');
  test.setTimeout(120_000);
  const source = process.env.PM_PERF_FILE
    ? decodeProjectContainer(readFileSync(process.env.PM_PERF_FILE)).document
    : JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!, 'utf8'));
  const project = source.proj || source;
  if (process.env.PM_PERF_EXTENSION) {
    cpSync(process.env.PM_PERF_EXTENSION, path.join(session.userData, 'extensions', path.basename(process.env.PM_PERF_EXTENSION)), { recursive: true });
  }
  if (process.env.PM_PERF_HISTORY) {
    mkdirSync(path.join(session.userData, 'store'), { recursive: true });
    cpSync(process.env.PM_PERF_HISTORY, path.join(session.userData, 'store', `projectState.${project.id}.json`));
  }
  if (process.env.PM_PERF_EXTENSION || process.env.PM_PERF_HISTORY) {
    await session.relaunch();
  }
  if (process.env.PM_PERF_EXTENSION) {
    await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.Kernel?.effects?.get('progressive-image-pixels'))).toBe(true);
  }
  if (process.env.PM_PERF_FILE) {
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
    }, process.env.PM_PERF_FILE);
    await session.page.evaluate(async () => { const PM = (window as any).PM; await PM.openProject(); PM.ProjectsScreen.hide(); PM.pause(); });
  } else {
    await session.page.evaluate(source => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: source }));
      PM.ProjectsScreen.hide(); PM.pause();
    }, project);
  }
  const cdp = await session.page.context().newCDPSession(session.page);
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, samples: number[] = [], renders: number[] = [], longTasks: number[] = [];
    const observer = new PerformanceObserver(list => { for (const entry of list.getEntries()) longTasks.push(entry.duration); }); observer.observe({ type: 'longtask' });
    const original = PM.GL.render;
    PM.GL.render = (...args: any[]) => { const start = performance.now(); const result = original(...args); renders.push(performance.now() - start); return result; };
    for (let i = 0; i < 80; i++) {
      const start = performance.now(); PM.setTime(((i * 37) % 83) / 83 * PM.proj.dur);
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); samples.push(performance.now() - start);
      await new Promise(resolve => setTimeout(resolve, 30));
    }
    observer.disconnect(); PM.GL.render = original;
    return { samples, renders, longTasks, layers: PM.proj.layers.length };
  });
  const profile = await cdp.send('Profiler.stop'); writeFileSync(info.outputPath('scrub.cpuprofile'), JSON.stringify(profile.profile));
  writeFileSync(info.outputPath('scrub.json'), JSON.stringify(result));
  console.log('SCRUB', JSON.stringify(result));
  expect(result.renders.sort((a, b) => a - b)[Math.floor(result.renders.length * .95)]).toBeLessThan(16.7);
  expect(Math.max(...result.renders)).toBeLessThan(100);
  if (process.env.PM_PERF_EXTENSION) {
    await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.GL.progs.get('fx:progressive-image-pixels')?.pr)).toBe(true);
    expect(await session.page.evaluate(() => (window as any).PM.GL.compileError('fx:progressive-image-pixels'))).toBeNull();
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('saving a large undo history yields to input and preserves the file', async ({ session }, info) => {
  test.setTimeout(120_000);
  const destination = path.join(session.userData, 'Responsive save.pmv');
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Responsive save' }) }));
    PM.hist.clear();
    PM.proj.largeSource = 'x'.repeat(36 * 1024 * 1024);
    PM.hist.do('Remove large source', () => { delete PM.proj.largeSource; });
  });
  await session.page.evaluate(async () => { await (window as any).PM.store.flush(); });
  const cdp = await session.page.context().newCDPSession(session.page);
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, gaps: number[] = [];
    let last = performance.now(), ticks = 0;
    const timer = setInterval(() => { const now = performance.now(); gaps.push(now - last); last = now; ticks++; }, 16);
    const start = performance.now(), ok = await PM.saveProject();
    clearInterval(timer);
    return { ok, ticks, gaps, elapsed: performance.now() - start };
  });
  writeFileSync(info.outputPath('save.json'), JSON.stringify(result)); console.log('SAVE', JSON.stringify(result));
  const profile = await cdp.send('Profiler.stop'); writeFileSync(info.outputPath('save.cpuprofile'), JSON.stringify(profile.profile));
  expect(result.ok).toBe(true); expect(result.ticks).toBeGreaterThan(3);
  expect(Math.max(...result.gaps)).toBeLessThan(100);
  const file = readFileSync(destination);
  expect(file.byteLength).toBeGreaterThan(36 * 1024 * 1024);
  const saved = decodeProjectContainer(file).document;
  const removed = saved.history.entries.flatMap((entry: any) => entry.backward)
    .find((patch: any) => patch.path?.join('.') === 'largeSource');
  expect(removed?.value.length).toBe(36 * 1024 * 1024);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
