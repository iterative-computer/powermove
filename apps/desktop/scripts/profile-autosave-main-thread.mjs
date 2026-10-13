// Measures the main process's longest event-loop stall while the editor
// autosaves, and during a native Save, for a large synthetic project.
//
//   npm run build && node scripts/profile-autosave-main-thread.mjs [--layers 884] [--history-batches 1] [--nudges 110] [--cycles 10]
//
// The app runs hidden on a disposable profile, the way the e2e harness
// launches it. Main samples its own event loop every 2 ms; a gap is time main
// could not run a timer, which is how long input, IPC, and frame forwarding
// waited. A hidden app's timers are coalesced by macOS, so gaps up to about
// 15 ms also appear at idle (reported as `idle`). Store IPC handlers are
// wrapped to attribute their synchronous time.
//
// 884 layers with 48 keyframes each make a 9.9 MB document. One history
// batch makes an undo history of about 42 MB on disk; three reach the
// editor's history budget (about 124 MB). The result is printed as JSON;
// nothing is written into the repository.
import { mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at === -1 ? fallback : Number(args[at + 1]);
};
const LAYERS = option('layers', 884), BATCHES = option('history-batches', 1), NUDGES = option('nudges', 110), CYCLES = option('cycles', 10);
const sleep = (ms) => new Promise(resolve => setTimeout(resolve, ms));

/** The e2e harness's hidden launch (e2e/helpers/app.ts), without its test fixtures. */
async function launchHidden() {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-autosave-profile-'));
  const env = { ...process.env, POWERMOVE_USER_DATA: userData, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1',
    CODEX_BINARY: path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'), POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await _electron.launch({ args: ['.'], cwd: desktop, env });
  await app.evaluate(({ dialog }) => {
    dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' });
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
  const pageErrors = [];
  const page = await app.firstWindow({ timeout: 60_000 });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.waitForFunction(() => Boolean(window.PM?.ProjectsScreen && window.PM?.mkProject));
  await page.evaluate(async () => { await window.PM.Kernel.loader.builtinsReady; });
  await page.evaluate(() => {
    const PM = window.PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Autosave profile' }) }));
    PM.ProjectsScreen.hide();
  });
  await page.waitForFunction(() => !window.PM.ProjectsScreen.isOpen && Boolean(window.PM.GL?.gl));
  return {
    app, page, userData, diagnostics: { pageErrors },
    async close() { await app.close().catch(() => undefined); await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); }
  };
}

const session = await launchHidden();
try {
  const built = await session.page.evaluate(({ LAYERS, BATCHES, NUDGES }) => {
    const PM = window.PM;
    const types = ['shape', 'text', 'solid'];
    const layers = [];
    for (let i = 0; i < LAYERS; i++) {
      const type = types[i % types.length];
      const d = type === 'text' ? { text: `Caption line ${i} with some words`, size: 24 + (i % 30), color: '#aadcff' }
        : type === 'shape' ? { shape: 'rect', w: 40 + (i % 200), h: 30 + (i % 120), radius: i % 12, color: '#dd5522' }
          : { w: 320, h: 180, color: '#336699' };
      const layer = PM.mkLayer(type, { name: `${type} ${i}`, d, p: { 'position.x': i % 640, 'position.y': (i * 7) % 360, rotation: i % 90, opacity: 50 + (i % 50) } });
      for (const channel of ['position.x', 'position.y', 'rotation', 'opacity']) {
        for (let k = 0; k < 12; k++) PM.setKeyOn(layer.p[channel], k * 0.25, (i * 13 + k * 17) % 300, 'ease', 30);
      }
      layers.push(layer);
    }
    PM.proj.layers = layers;
    PM.ProjectIndex.invalidate();
    PM.touch();
    PM.hist.clear();
    // Undo keeps every layer an edit added or removed. Batches of unique
    // layers that come and go give the history the size of a long session.
    for (let g = 0; g < BATCHES; g++) {
      const batch = PM.proj.layers.slice(0, 90).map((layer, i) => ({ ...structuredClone(layer), id: PM.uid('L'), name: `Batch ${g}.${i}` }));
      PM.hist.do(`Add batch ${g}`, () => { PM.proj.layers = [...PM.proj.layers, ...batch]; PM.ProjectIndex.invalidate(); });
      PM.hist.do(`Remove batch ${g}`, () => { PM.proj.layers = PM.proj.layers.slice(0, -batch.length); PM.ProjectIndex.invalidate(); });
    }
    for (let h = 0; h < NUDGES; h++) {
      PM.hist.do(`Nudge ${h}`, () => { PM.proj.layers[h % PM.proj.layers.length].p['position.x'].v += 1; });
    }
    return { id: PM.proj.id, documentBytes: JSON.stringify(PM.proj).length };
  }, { LAYERS, BATCHES, NUDGES });

  const storeDir = path.join(session.userData, 'store');
  const waitForQuiet = async () => {
    // Let the initial checkpoint autosave of the generated project finish.
    await session.page.evaluate(() => new Promise(resolve => { const off = window.PM.bus.on('project:recovered', () => { off(); resolve(); }); }));
    await sleep(1500);
  };
  await waitForQuiet();

  await session.app.evaluate(() => {
    const state = { last: performance.now(), gaps: [] };
    state.timer = setInterval(() => {
      const now = performance.now();
      const gap = now - state.last;
      if (gap > 4) state.gaps.push([Date.now(), gap]);
      state.last = now;
    }, 2);
    globalThis.__autosaveProfile = state;
  });
  // Attribute main's time to store writes: wrap the store's IPC listeners and
  // invoke handlers (the latter through Electron's private handler map).
  await session.app.evaluate(({ ipcMain }) => {
    const calls = [];
    globalThis.__autosaveProfile.calls = calls;
    const keyOf = (payload) => (payload && typeof payload.key === 'string' ? payload.key.replace(/\.[A-Za-z0-9_-]+$/, '.<id>') : '');
    for (const channel of ['store:set', 'store:delete']) {
      for (const listener of ipcMain.listeners(channel)) {
        ipcMain.removeListener(channel, listener);
        ipcMain.on(channel, (event, payload) => {
          const start = performance.now();
          try { return listener(event, payload); } finally { calls.push([Date.now(), channel, keyOf(payload), performance.now() - start]); }
        });
      }
    }
    const handlers = ipcMain._invokeHandlers;
    for (const channel of ['store:set-serialized', 'store:flush', 'project:save-chunk', 'project:save-finish', 'project:save-begin']) {
      const handler = handlers?.get(channel);
      if (!handler) continue;
      handlers.set(channel, (event, payload) => {
        const start = performance.now();
        let result;
        try { result = handler(event, payload); } finally { calls.push([Date.now(), channel, keyOf(payload), performance.now() - start]); }
        return result;
      });
    }
  });
  const windowMax = async (from, to) => session.app.evaluate((_electron, range) => {
    // Each gap is [wall time it ended, length]; keep the ones overlapping the range.
    const gaps = globalThis.__autosaveProfile.gaps.filter(([end, gap]) => end >= range.from && end - gap <= range.to).map(([, gap]) => gap);
    const calls = (globalThis.__autosaveProfile.calls || []).filter(([at]) => at >= range.from && at <= range.to);
    const writes = {};
    for (const [, channel, key, ms] of calls) {
      const name = `${channel} ${key}`.trim();
      writes[name] = Math.max(writes[name] || 0, Math.round(ms * 10) / 10);
    }
    return { max: gaps.length ? Math.max(...gaps) : 0, over16: gaps.filter(gap => gap > 16.7).length, over50: gaps.filter(gap => gap > 50).length, longestSyncHandlerMs: writes };
  }, { from, to });

  const idleFrom = Date.now();
  await sleep(4000);
  const idle = await windowMax(idleFrom, Date.now());

  const cycles = [];
  for (let cycle = 0; cycle < CYCLES; cycle++) {
    const from = Date.now();
    // Every 50th revision checkpoints the whole project; the others journal.
    await session.page.evaluate(async (checkpoint) => {
      const PM = window.PM;
      const done = new Promise(resolve => { const off = PM.bus.on('project:recovered', () => { off(); resolve(); }); });
      const base = 50 * (Math.floor(Number(PM.proj.revision || 0) / 50) + 1);
      PM.proj.revision = checkpoint ? base : base + 1;
      PM.hist.do('Profile nudge', () => { PM.proj.layers[0].p['position.y'].v += 1; });
      await done;
    }, cycle % 2 === 0);
    await sleep(600);
    const measured = await windowMax(from, Date.now());
    cycles.push({ kind: Object.keys(measured.longestSyncHandlerMs).some(name => name.includes('project.<id>')) ? 'checkpoint' : 'journal', ...measured });
  }

  const destination = path.join(session.userData, 'Profile.pmv');
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
  const saves = [];
  for (let i = 0; i < 3; i++) {
    const from = Date.now();
    const ok = await session.page.evaluate(async (i) => {
      const PM = window.PM;
      PM.hist.do('Profile save edit', () => { PM.proj.layers[1].p['position.y'].v += i + 1; });
      return PM.saveProject();
    }, i);
    await sleep(300);
    saves.push({ ok, ...(await windowMax(from, Date.now())) });
  }
  await session.app.evaluate(() => clearInterval(globalThis.__autosaveProfile.timer));
  // Main's memory once the saves settle: parsed values live on the V8 heap,
  // encoded bytes in ArrayBuffers.
  await sleep(1000);
  const mainMemoryMiB = await session.app.evaluate(() => {
    const usage = process.memoryUsage();
    return Object.fromEntries(['rss', 'heapUsed', 'external', 'arrayBuffers'].map(name => [name, Math.round(usage[name] / 1048576)]));
  });

  const files = {};
  for (const name of await readdir(storeDir)) {
    if (name.includes(built.id)) files[name.replace(built.id, '<id>')] = (await stat(path.join(storeDir, name))).size;
  }
  const pmv = (await stat(destination).catch(() => null))?.size ?? null;
  const summarize = (rows) => !rows.length ? null : ({ maxMs: Math.max(...rows.map(row => row.max)), medianMaxMs: [...rows.map(row => row.max)].sort((a, b) => a - b)[Math.floor(rows.length / 2)], over50: rows.reduce((n, row) => n + row.over50, 0) });
  console.log(JSON.stringify({
    layers: LAYERS, historyBatches: BATCHES, nudges: NUDGES, documentBytes: built.documentBytes, storeFiles: files, pmvBytes: pmv,
    idle,
    autosave: { all: summarize(cycles), checkpoint: summarize(cycles.filter(row => row.kind === 'checkpoint')), journal: summarize(cycles.filter(row => row.kind === 'journal')), cycles },
    save: { ...summarize(saves), saves },
    mainMemoryMiB,
    rendererErrors: session.diagnostics.pageErrors.length
  }, null, 2));
} finally {
  await session.close();
}
