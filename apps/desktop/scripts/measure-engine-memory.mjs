// Prebuild the app/addons, then run this command under pm-electron-lock.
// PM_MEMORY_OUTPUT must be outside the checkout; PM_MEMORY_PROJECT names a
// corpus loaded.json at runtime. No corpus contents enter source control.
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir, loadavg } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { _electron } from 'playwright';

const exec = promisify(execFile);
const desktop = path.resolve(import.meta.dirname, '..');
const checkout = path.resolve(desktop, '../..');
const output = path.resolve(process.env.PM_MEMORY_OUTPUT ?? '');
const outputRelative = path.relative(checkout, output);
if (!process.env.PM_MEMORY_OUTPUT || !(outputRelative === '..' || outputRelative.startsWith(`..${path.sep}`) || path.isAbsolute(outputRelative))) {
  throw new Error('PM_MEMORY_OUTPUT must name an evidence directory outside the checkout.');
}
const project = JSON.parse(await readFile(process.env.PM_MEMORY_PROJECT, 'utf8'));
if (Object.keys(project.assets ?? {}).length) throw new Error('Use an asset-free corpus project; this isolated harness does not install media bindings.');
const count = Number(process.env.PM_MEMORY_SAMPLES ?? 5);
const variants = (process.env.PM_MEMORY_VARIANTS ?? 'off,after').split(',');
if (!Number.isInteger(count) || count < 1 || variants.some(v => !['off', 'before', 'nativeBefore', 'compatibility', 'homeControl', 'pipelines', 'after'].includes(v))) throw new Error('Invalid measurement settings.');
await mkdir(output, { recursive: true });
// Every after sample uses the same inode even if another local build finishes.
const measuredAddon = path.join(output, 'measured.node');
const measuredBridge = path.join(output, 'measured-bridge.node');
await copyFile(path.join(desktop, 'native/pm_node.node'), measuredAddon);
await copyFile(path.join(desktop, 'native/pm_bridge.node'), measuredBridge);
const sha256 = async file => createHash('sha256').update(await readFile(file)).digest('hex');
const appFiles = ['out/main/index.js', 'out/main/editor.js', 'out/main/engine-process.js', 'out/renderer/index.html'];
const appHashes = Object.fromEntries(await Promise.all(appFiles.map(async file => [file, await sha256(path.join(desktop, file))])));
await writeFile(path.join(output, 'artifacts.json'), JSON.stringify({
  appHashes,
  afterSha256: await sha256(measuredAddon),
  bridgeSha256: await sha256(measuredBridge),
  beforeSha256: process.env.PM_MEMORY_BASELINE_ADDON ? await sha256(process.env.PM_MEMORY_BASELINE_ADDON) : null
}, null, 2));
const samples = [];
let app;
let interrupted = false;
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {
  interrupted = true;
  void app?.evaluate(({ app }) => app.exit(0)).catch(() => {});
});

const settleMs = Number(process.env.PM_MEMORY_SETTLE_MS ?? 3000);
const seriesEveryMs = 500;

// Physical footprint and owned graphics bytes per process (one footprint call).
async function footprints(pids) {
  const file = path.join(output, 'series-footprint.json');
  let json;
  try {
    await exec('footprint', [...pids.flatMap(p => ['-p', String(p.pid)]), '-f', 'bytes', '-j', file], { timeout: 30_000, maxBuffer: 16 * 1024 ** 2 });
    json = JSON.parse(await readFile(file, 'utf8'));
  } catch {
    return [];
  } finally {
    await rm(file, { force: true });
  }
  return pids.map(p => {
    const f = json.processes?.find(f => f.pid === p.pid);
    const graphics = f ? Object.entries(f.categories).filter(([k]) => k.includes('(graphics)')).reduce((n, [, c]) => n + c.dirty, 0) : null;
    return { pid: p.pid, role: p.role, footprintBytes: f?.footprint ?? null, graphicsBytes: graphics };
  });
}

async function capture(state, prefix, first, variant) {
  // Match the quiescent editor and force the same GC on both paths.
  const page = app.windows()[0];
  const cdp = await page.context().newCDPSession(page);
  let rendererHeap;
  try {
    await cdp.send('HeapProfiler.collectGarbage');
    rendererHeap = await cdp.send('Runtime.getHeapUsage');
  } finally { await cdp.detach(); }
  // Any Metal submission charges a process ~160-400 MiB of owned graphics
  // memory until about 1.5 s after its last GPU work (docs/engine/memory.md,
  // "Attribution and remaining boundaries"). A series shows whether a process is still active
  // when the capture is taken. This fixed delay alone does not prove rest;
  // inspect presenter activity and allocation counters before claiming it.
  const series = [];
  const seriesStart = performance.now();
  while (performance.now() - seriesStart < settleMs) {
    const pids = await app.evaluate(({ app }) => app.getAppMetrics().map(m => ({ pid: m.pid, role: m.serviceName || m.type })));
    series.push({ ms: Math.round(performance.now() - seriesStart), processes: await footprints(pids) });
    await page.waitForTimeout(seriesEveryMs);
  }
  const processes = await app.evaluate(({ app }) => app.getAppMetrics().map(m => ({
    pid: m.pid, role: m.serviceName || m.type, workingSetBytes: m.memory.workingSetSize * 1024
  })));
  const file = path.join(output, `${prefix}-${state}-footprint.json`);
  const args = processes.flatMap(p => ['-p', String(p.pid)]);
  let footprint;
  try {
    await exec('footprint', [...args, '--swapped', '-f', 'bytes', '-j', file], { timeout: 30_000, maxBuffer: 16 * 1024 ** 2 });
    footprint = JSON.parse(await readFile(file, 'utf8'));
    if (footprint.errors.length) throw new Error(JSON.stringify(footprint.errors));
  } catch (error) {
    throw new Error(`footprint failed: ${error.message}`);
  }
  for (const p of processes) {
    const f = footprint.processes.find(f => f.pid === p.pid);
    if (!f) throw new Error(`footprint omitted process ${p.pid}`);
    p.physicalFootprintBytes = f.footprint;
    p.categories = f.categories;
    p.dirtyBytes = Object.values(f.categories).reduce((n, c) => n + c.dirty, 0);
    p.swappedOrCompressedBytes = Object.values(f.categories).reduce((n, c) => n + c.swapped, 0);
    // macOS footprint combines compressed and swapped pages. Zero proves
    // both are zero; nonzero must remain combined rather than invented.
    p.compressedBytes = p.swappedOrCompressedBytes === 0 ? 0 : null;
    p.swappedBytes = p.swappedOrCompressedBytes === 0 ? 0 : null;
    const threads = await exec('ps', ['-M', '-p', String(p.pid), '-o', 'pid='], { timeout: 10_000 });
    p.threadCount = threads.stdout.trim().split('\n').filter(line => line && !/^USER\s/.test(line)).length;
    if (first) {
      const result = await exec('vmmap', ['--summary', String(p.pid)], { timeout: 30_000, maxBuffer: 8 * 1024 ** 2 });
      await writeFile(path.join(output, `${prefix}-${state}-${p.pid}-vmmap.txt`), result.stdout);
    }
  }
  const engine = await app.evaluate(async () => {
    const host = globalThis.__powermoveNativeEngine?.engine;
    if (!host) return null;
    return { status: host.status(), interactive: await host.report('interactive'), job: await host.report('job') };
  });
  const rendererEngine = await page.evaluate(async () => ({
    wasmHeap: window.PM.EngineDoc?.wasm?.heap() ?? null,
    legacyContextCreated: Boolean(window.PM.GL?.peekContext?.()),
    compatibilityFirstUseMs: window.PM.GL?.compatFirstUseMs ?? null,
    legacyAllocations: window.PM.GL?.allocationStats?.() ?? null,
    nativePresentation: await window.PM.Kernel.services.get('viewer')?.enginePresentation?.memoryStats?.() ?? null
  }));
  if (['after', 'nativeBefore', 'compatibility', 'homeControl'].includes(variant)
      && rendererEngine.legacyContextCreated !== (variant === 'compatibility')) {
    throw new Error(`Unexpected compatibility GL state in ${variant}/${state}.`);
  }
  if (engine) for (const [kind, status] of Object.entries(engine.status)) {
    const process = processes.find(p => p.pid === status.pid);
    if (process) process.role = `Powermove ${kind} engine`;
  }
  // footprint's combined summary accounts shared regions across these PIDs.
  return { state, load: loadavg(), processes, engine, rendererHeap, rendererEngine, settleMs, series,
    workingSetBytes: processes.reduce((n, p) => n + p.workingSetBytes, 0),
    physicalFootprintBytes: footprint['total footprint'],
    summedProcessFootprintBytes: processes.reduce((n, p) => n + p.physicalFootprintBytes, 0),
    sharedAccounting: footprint.summary };
}

for (let round = 0; round < count; round++) {
  // Reverse the pair order on alternate rounds to reduce order bias.
  for (const variant of round % 2 ? [...variants].reverse() : variants) {
    if (interrupted) throw new Error('Measurement interrupted.');
    const profile = await mkdtemp(path.join(tmpdir(), 'pm-engine-memory-'));
    const prefix = `${round + 1}-${variant}`;
    try {
      const env = { ...process.env, POWERMOVE_NEW_ENGINE: variant === 'off' ? '0' : '1',
        POWERMOVE_PM_BRIDGE: measuredBridge,
        POWERMOVE_ENGINE_EAGER_JOBS: ['before', 'pipelines'].includes(variant) ? '1' : '0',
        POWERMOVE_ENGINE_RUNTIME_MEMORY: '1', POWERMOVE_ENGINE_ALLOCATION_INVENTORY: '1',
        POWERMOVE_USER_DATA: profile, POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_DEVTOOLS: '0',
        CODEX_BINARY: path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'), POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
      // `nativeBefore` isolates addon changes with the current renderer and
      // the same on-demand job policy; `before` retains the eager control.
      if (variant === 'before' || variant === 'nativeBefore') {
        if (!process.env.PM_MEMORY_BASELINE_ADDON) throw new Error('PM_MEMORY_BASELINE_ADDON is required for before.');
        env.POWERMOVE_ENGINE_ADDON = process.env.PM_MEMORY_BASELINE_ADDON;
      } else if (['after', 'compatibility', 'homeControl', 'pipelines'].includes(variant)) env.POWERMOVE_ENGINE_ADDON = measuredAddon;
      else delete env.POWERMOVE_ENGINE_ADDON;
      delete env.ELECTRON_RUN_AS_NODE;
      app = await _electron.launch({ args: ['.', '--force-color-profile=srgb'], cwd: desktop, env, timeout: 90_000 });
      await app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); });
      const page = await app.firstWindow();
      await page.waitForFunction(() => Boolean(window.PM?.ProjectsScreen), undefined, { timeout: 90_000 });
      await page.evaluate(async () => { await window.PM.Kernel.loader.builtinsReady; });
      if (variant !== 'off') await app.evaluate(async () => { await globalThis.__powermoveNativeEngine.engine.ensureReady('interactive'); });
      if (process.env.PM_MEMORY_HOME_SUSPEND_GATE === '1' && variant !== 'off') {
        await page.evaluate(enabled => {
          const connection = window.PM.EngineConnection;
          if (typeof connection?.setHomeSuspendForDiagnostics !== 'function') throw new Error('Home suspension diagnostics are missing.');
          connection.setHomeSuspendForDiagnostics(enabled);
        }, variant !== 'homeControl');
      }
      if (variant === 'compatibility') {
        // Same native addon and viewer, with the older API's context already
        // allocated. Isolate its cost without changing the rendering route.
        await page.waitForFunction(() => Boolean(window.PM.GL?.canvas));
        await page.evaluate(() => {
          if (!window.PM.GL.gl) throw new Error('The compatibility context failed.');
        });
      }
      const states = [await capture('idle', prefix, round === 0, variant)];
      const emptyStarted = performance.now();
      await page.evaluate(() => {
        const PM = window.PM;
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Memory measurement' }) }));
        PM.ProjectsScreen.hide();
      });
      await page.waitForFunction(() => !!window.PM.Kernel.services.get('viewer')?.stage?.isConnected, undefined, { timeout: 60_000 });
      if (variant !== 'off') await page.waitForFunction(() => window.PM.EngineConnection?.status().route === 'engine', undefined, { timeout: 60_000 });
      await page.waitForTimeout(1000);
      const emptyReadyMs = performance.now() - emptyStarted;
      const firstEmptySnapshot = await page.evaluate(async () => {
        const start = performance.now();
        const image = await window.PM.Export.snapshotAsync(0, 320);
        if (typeof image !== 'string' || !image.startsWith('data:image/')) throw new Error('The empty snapshot failed.');
        return { ms: performance.now() - start, encodedBytes: image.length };
      });
      states.push(await capture('empty', prefix, round === 0, variant));
      const projectStarted = performance.now();
      await page.evaluate(project => { window.PM.replaceProject(project); }, project);
      if (variant !== 'off') await page.waitForFunction(() => window.PM.EngineConnection?.status().route === 'engine', undefined, { timeout: 60_000 });
      await page.waitForTimeout(1500);
      const projectReadyMs = performance.now() - projectStarted;
      const firstCorpusSnapshot = await page.evaluate(async () => {
        const start = performance.now();
        const image = await window.PM.Export.snapshotAsync(0, 320);
        if (typeof image !== 'string' || !image.startsWith('data:image/')) throw new Error('The corpus snapshot failed.');
        return { ms: performance.now() - start, encodedBytes: image.length };
      });
      states.push(await capture('corpus', prefix, round === 0, variant));
      // Sample a snapshot after the fixed capture delay, then a warm one.
      // The inventory and rest counters establish whether targets retired.
      const snapshotMs = () => page.evaluate(async () => {
        const start = performance.now();
        const image = await window.PM.Export.snapshotAsync(0, 320);
        if (typeof image !== 'string' || !image.startsWith('data:image/')) throw new Error('The snapshot failed.');
        return performance.now() - start;
      });
      const restSnapshot = { afterRestMs: await snapshotMs(), warmMs: await snapshotMs() };
      const restReport = variant === 'off' ? null : await app.evaluate(async () => {
        const report = await globalThis.__powermoveNativeEngine.engine.report('interactive');
        return { restReleases: report?.presenter?.restReleases ?? null, restReleasedBytes: report?.presenter?.restReleasedBytes ?? null };
      });
      if (process.env.PM_MEMORY_CLOSED === '1') {
        // Back to an empty project: what the corpus project leaves behind.
        await page.evaluate(() => { window.PM.replaceProject(window.PM.mkProject({ name: 'Memory measurement' })); });
        if (variant !== 'off') await page.waitForFunction(() => window.PM.EngineConnection?.status().route === 'engine', undefined, { timeout: 60_000 });
        await page.waitForTimeout(1500);
        states.push(await capture('closed', prefix, round === 0, variant));
        if (process.env.PM_MEMORY_HOME_CLOSED === '1') {
          // Also exercise the actual last-tab close. This profile belongs
          // only to this measurement, so discard its generated projects.
          // replaceProject changes the document without adding its new id
          // to the tab strip. Register that document through the same open
          // event used above before exercising the real tab-close path.
          await page.evaluate(() => {
            window.dispatchEvent(new CustomEvent('pm-open-project', { detail: window.PM.proj }));
          });
          await page.waitForFunction(() => window.PM.Tabs.list().includes(window.PM.proj.id));
          await page.evaluate(async () => {
            window.PM.confirmCloseProject = async () => true;
            for (const id of window.PM.Tabs.list()) await window.PM.Tabs.close(id);
          });
          await page.waitForFunction(() => window.PM.isHomeProject() && window.PM.ProjectsScreen.isOpen);
          await page.waitForTimeout(2500);
          if (process.env.PM_MEMORY_HOME_SUSPEND_GATE === '1' && variant !== 'off' && variant !== 'homeControl') {
            await page.waitForFunction(async () => {
              const stats = await window.PM.Kernel.services.get('viewer')?.enginePresentation?.memoryStats?.();
              return stats?.suspended === true && stats?.channelOpen === false && stats?.pendingDrains === 0 && stats?.painter?.trimmed === true;
            }, undefined, { timeout: 10000 });
          }
          const homeClosed = await capture('homeClosed', prefix, round === 0, variant);
          if (process.env.PM_MEMORY_HOME_SUSPEND_GATE === '1' && variant !== 'off' && variant !== 'homeControl') {
            const inventory = homeClosed.engine?.interactive?.presenter?.allocations;
            const presentation = homeClosed.rendererEngine.nativePresentation;
            if (!inventory || inventory.gpuCreated || inventory.colorSurfaceBytes !== 0 || inventory.workingTargetBytes !== 0
                || presentation?.imports?.held !== 0 || presentation?.painter?.trimmed !== true) {
              throw new Error('The suspended home viewer retained its device, pool, imports, or full-size painter.');
            }
          }
          states.push(homeClosed);
        }
      }
      const agentPrewarm = variant === 'off' ? null : await app.evaluate(async ({ BrowserWindow }, project) => {
        const installed = globalThis.__powermoveNativeEngine;
        const start = performance.now();
        await installed.agentJobs.prepareRun('memory-agent-run');
        const startupMs = performance.now() - start;
        try {
          const requested = performance.now();
          await installed.engine.ensureReady('job');
          const readyOnUseMs = performance.now() - requested;
          const owner = BrowserWindow.getAllWindows()[0].webContents.id;
          const render = async () => {
            const began = performance.now();
            const result = await installed.agentJobs.renderFrames(owner, 'memory-agent-run', {
              project, times: [0], width: 320, revision: 1
            });
            if (result.status !== 'rendered' || result.images.length !== 1) throw new Error('The agent frame failed.');
            return performance.now() - began;
          };
          return { startupMs, readyOnUseMs, firstFrameMs: await render(), warmFrameMs: await render() };
        } finally { await installed.agentJobs.finishRun('memory-agent-run'); }
      }, project);
      const firstJobReadyMs = agentPrewarm?.startupMs ?? null;
      samples.push({ round: round + 1, variant, states, emptyReadyMs, projectReadyMs, firstJobReadyMs, agentPrewarm, firstEmptySnapshot, firstCorpusSnapshot, restSnapshot, restReport });
      await writeFile(path.join(output, 'samples.json'), JSON.stringify(samples, null, 2));
      process.stdout.write(`${prefix} captured\n`);
      let trace;
      if (round === 0 && variant !== 'off' && process.env.PM_MEMORY_TRACE === '1') {
        const pid = states[0].engine.status.interactive.pid;
        trace = exec('xcrun', ['xctrace', 'record', '--template', 'Allocations', '--instrument', 'VM Tracker',
          '--attach', String(pid), '--time-limit', '5s', '--no-prompt', '--output', path.join(output, `${prefix}.trace`)],
        { timeout: 30_000, killSignal: 'SIGKILL', maxBuffer: 8 * 1024 ** 2 }).then(
          result => writeFile(path.join(output, `${prefix}-trace.log`), result.stdout + result.stderr),
          error => writeFile(path.join(output, `${prefix}-trace.log`), String(error))
        );
      }
      await trace;
      for (const file of appFiles) {
        if (await sha256(path.join(desktop, file)) !== appHashes[file]) throw new Error(`The app build changed during measurement: ${file}`);
      }

    } catch (error) {
      const diagnostics = {
        error: String(error),
        renderer: await app?.windows()[0]?.evaluate(() => ({
          status: window.PM.EngineConnection?.status(),
          failures: window.PM.EngineDoc?.failures(),
          legacyContextCreated: Boolean(window.PM.GL?.peekContext?.()),
          home: window.PM.ProjectsScreen?.isOpen
        })).catch(cause => String(cause)),
        engines: await app?.evaluate(async () => {
          const host = globalThis.__powermoveNativeEngine?.engine;
          return host ? { status: host.status(), interactive: await host.report('interactive') } : null;
        }).catch(cause => String(cause))
      };
      await writeFile(path.join(output, `${prefix}-failure.json`), JSON.stringify(diagnostics, null, 2));
      throw error;
    } finally {
      if (app) {
        const child = app.process();
        await app.evaluate(({ app }) => app.exit(0)).catch(() => {});
        // close() waits for the process; kill is only a cleanup backstop.
        await app.close().catch(() => { if (child.exitCode === null) child.kill('SIGKILL'); });
        app = undefined;
      }
      await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    }
  }
}
