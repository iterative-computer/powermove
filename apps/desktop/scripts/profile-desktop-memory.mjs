// Compare built desktop trees using disposable copies of one saved store.
// node scripts/profile-desktop-memory.mjs /desktop/root /source/store
import { _electron } from 'playwright';
import { cp, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { once } from 'node:events';

const [root, source] = process.argv.slice(2);
if (!root || !source) throw new Error('Expected desktop root and source store directory');
const profile = await mkdtemp(join(tmpdir(), 'powermove-footprint-'));
let app;
async function stop() {
  if (!app) return;
  const child = app.process();
  const exited = child.exitCode === null && child.signalCode === null ? once(child, 'exit') : Promise.resolve();
  await app.evaluate(({ app }) => app.exit(0)).catch(() => {});
  await exited;
  app = undefined;
}
try {
  await cp(source, join(profile, 'store'), { recursive: true });
  await writeFile(join(profile, 'store/restoreWindows.json'), 'false');
  await writeFile(join(profile, 'store/openWindows.json'), '[]');
  const env = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1',
    CODEX_BINARY: join(resolve(root), 'src/main/codex/__fixtures__/fake-codex-app-server.sh'), POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
  delete env.ELECTRON_RUN_AS_NODE;
  const samples = [];
  for (let pass = 0; pass < Number(process.argv[4] || 1); pass++) {
  const started = performance.now();
  app = await _electron.launch({ args: ['.', '--enable-precise-memory-info'], cwd: resolve(root), env, timeout: 120_000 });
  const page = await app.firstWindow();
  await page.waitForFunction(() => Boolean(window.PM?.ProjectsScreen), undefined, { timeout: 120_000 });
  const readyMs = performance.now() - started;
  await page.waitForTimeout(1500);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('HeapProfiler.collectGarbage');
  const renderer = await cdp.send('Runtime.getHeapUsage');
  const main = await app.evaluate(async ({ app }) => {
    const v8 = process.getBuiltinModule('v8'), vm = process.getBuiltinModule('vm');
    v8.setFlagsFromString('--expose_gc');
    const collect = vm.runInNewContext('gc');
    collect();
    return { heapUsed: v8.getHeapStatistics().used_heap_size, memory: await process.getProcessMemoryInfo(),
      processes: app.getAppMetrics().map(({ pid, type, memory }) => ({ pid, type, memory })) };
  });
  const resources = await page.evaluate(() => ({ memory: window.PM.Memory.stats(),
    domNodes: document.querySelectorAll('*').length, home: window.PM.ProjectsScreen.isOpen,
    layers: window.PM.proj.layers.length, history: window.PM.hist.list().length }));
  const footprints = main.processes.filter(p => ['Browser', 'Tab', 'GPU'].includes(p.type)).map(p => {
    try {
      const summary = execFileSync('vmmap', ['-summary', String(p.pid)], { encoding: 'utf8', timeout: 15_000, stdio: ['ignore', 'pipe', 'ignore'] });
      return { type: p.type, footprint: /^Physical footprint:\s+(.+)$/m.exec(summary)?.[1]?.trim(),
        peak: /^Physical footprint \(peak\):\s+(.+)$/m.exec(summary)?.[1]?.trim() };
    } catch { return { type: p.type, unavailable: true }; }
  });
  samples.push({ pass: pass + 1, readyMs, renderer, main: { heapUsed: main.heapUsed, memory: main.memory }, resources, footprints });
  await page.evaluate(() => window.PM.store.flush());
  await stop();
  }
  console.log(JSON.stringify(samples.length === 1 ? samples[0] : samples, null, 2));
} finally {
  // A copied recovery project may be dirty; never show a native save dialog
  // or write it back merely to finish a read-only disposable-profile sample.
  await stop();
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
