#!/usr/bin/env node
/* Spike S5's targets measured in the app (docs/rust-engine.md, section 26,
   Phase 1 gate: "S5 targets met in the app"): the built editor, hidden and
   unfocusable, with the native evaluator on, the 884-layer synthetic project
   of S5 open (scripts/lib/synth-project.mjs), and the real write path:
   the kernel's evP through the identity registry, edits through the Edit
   pipeline and history, flushed through the write barrier into pm-wasm.

     bun run build                       # out/ must exist (any flag value)
     node scripts/pm-wasm-bench.mjs [--rounds N]

   Prints JSON. Targets: evP cached under 1 µs; patch apply under 0.2 ms for
   a property edit and under 2 ms for a structural edit; memory reported. */
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';
import { project884 } from './lib/synth-project.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const appDir = path.resolve(here, '..');
const rounds = Number(process.argv[process.argv.indexOf('--rounds') + 1]) || 200;

const userData = await mkdtemp(path.join(tmpdir(), 'pm-wasm-bench-'));
// The e2e harness's offline agent runtime: no real app-server outlives the app.
const env = {
  ...process.env,
  CODEX_BINARY: path.join(appDir, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
  POWERMOVE_FAKE_CHATGPT_STATUS: 'connected',
  POWERMOVE_USER_DATA: userData, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1',
  POWERMOVE_DISABLE_RUNTIME_UPDATES: '1'
};
delete env.ELECTRON_RUN_AS_NODE;
const app = await _electron.launch({ args: ['.'], cwd: appDir, env });
// A stuck page must not leave a hidden Electron behind.
const quit = async () => {
  const closed = app.close().then(() => true, () => false);
  if (!(await Promise.race([closed, new Promise(r => setTimeout(() => r(false), 10_000))]))) app.process().kill('SIGKILL');
};
const deadline = setTimeout(() => { console.error('[bench] timed out'); quit().finally(() => process.exit(3)); }, 20 * 60_000);
try {
  let page = await app.firstWindow();
  await page.waitForFunction(() => Boolean(window.PM?.ProjectsScreen && window.PM?.mkProject));
  // The flag for this profile only, then a reload so it installs.
  await page.evaluate(() => localStorage.setItem('powermove:native-evaluator', 'on'));
  await page.reload();
  await page.waitForFunction(() => Boolean(window.PM?.ProjectsScreen && window.PM?.EngineDoc));
  const ready = await page.evaluate(() => window.PM.EngineDoc.whenReady);
  if (!ready) throw new Error('pm-wasm did not load in the renderer');

  page.on('console', (m) => { if (m.text().startsWith('[bench]')) console.error(m.text()); });
  const synthetic = project884();
  // The synthetic project keeps every composition field at the root (the stored form).
  const project = { ...synthetic, id: 'P-bench', compId: 'C-bench', compName: 'Bench', comps: {}, assets: {}, markers: [], work: [0, synthetic.dur ?? 60] };
  const result = await page.evaluate(async ({ project, rounds }) => {
    const PM = window.PM;
    const doc = PM.EngineDoc;
    await PM.Kernel.loader.builtinsReady;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    await new Promise(r => setTimeout(r, 500));
    const barrier = doc.barrier, wasm = doc.wasm, api = doc.api;
    const phase = (name) => console.log(`[bench] ${name} ${performance.now().toFixed(0)}`);
    phase('open');
    const quantile = (xs, q) => { const s = [...xs].sort((a, b) => a - b); return s[Math.min(s.length - 1, Math.floor(q * s.length))]; };
    const stats = (xs) => ({ p50: quantile(xs, 0.5), p95: quantile(xs, 0.95), mean: xs.reduce((a, b) => a + b, 0) / xs.length, n: xs.length });

    // Time pm-wasm's own apply (decode + id-keyed reconciliation) per batch.
    const applies = [];
    const apply = wasm.apply.bind(wasm);
    wasm.apply = (body, base, rev) => { const t0 = performance.now(); const r = apply(body, base, rev); applies.push(performance.now() - t0); return r; };
    const flushes = [];
    const flush = () => { const t0 = performance.now(); barrier.flush(); flushes.push(performance.now() - t0); };

    // Load (resync of the whole document).
    const t0 = performance.now(); barrier.resync(); const loadMs = performance.now() - t0;
    const layers = PM.proj.layers;
    const channels = [];
    for (const L of layers) for (const key of Object.keys(L.p || {})) channels.push([L, L.p[key], key]);
    const animated = channels.filter(([, p]) => p.kf?.length);

    const timeEvP = (list, T, passes) => {
      for (const [L, p, k] of list) api.evP(L, p, T, k); // warm: intern and cache
      const t = performance.now();
      for (let i = 0; i < passes; i++) for (const [L, p, k] of list) api.evP(L, p, T, k);
      return (performance.now() - t) * 1e6 / (passes * list.length);
    };
    const timeLegacy = (list, T, passes) => {
      const t = performance.now();
      for (let i = 0; i < passes; i++) for (const [L, p, k] of list) PM.evP(L, p, T, k);
      return (performance.now() - t) * 1e6 / (passes * list.length);
    };
    phase('evP');
    const evPAllNs = timeEvP(channels, 1.5, 20);
    const evPAnimatedNs = timeEvP(animated, 1.5, 200);
    const legacyAllNs = timeLegacy(channels, 1.5, 20);
    // A new time each pass: the curve is solved again.
    let tNew = 0; const tN = performance.now();
    for (let i = 0; i < 50; i++) { tNew = 2 + i * 0.0137; for (const [L, p, k] of animated) api.evP(L, p, tNew, k); }
    const evPNewTimeNs = (performance.now() - tN) * 1e6 / (50 * animated.length);
    // The batch path (numeric ids, shared Float64Array).
    const ids = channels.map(([L, , k]) => wasm.intern({ comp: PM.proj.compId, layer: L.id, key: k, via: 'channel' }));
    wasm.evBatch(ids, 1.5);
    const tb = performance.now();
    for (let i = 0; i < 50; i++) wasm.evBatch(ids, 1.5);
    const batchNs = (performance.now() - tb) * 1e6 / (50 * ids.length);

    phase('property edits');
    // Property edits through the Edit pipeline (history and the barrier).
    applies.length = 0; flushes.length = 0;
    const target = layers[Math.floor(layers.length / 2)];
    for (let i = 0; i < rounds; i++) {
      PM.Edit.apply({ type: 'set_property', target: target.id, path: 'position.x', value: 100 + i });
      flush();
    }
    const property = { apply: stats(applies.slice()), flush: stats(flushes.slice()) };
    // Back-to-back: `rounds` edits, one flush each (the mean, below timer resolution).
    applies.length = 0;
    const tp = performance.now();
    for (let i = 0; i < rounds; i++) { target.p['position.y'].v = i; PM.touch([['layers', layers.indexOf(target), 'p', 'position.y', 'v']]); barrier.flush(); }
    const propertyBackToBackMs = (performance.now() - tp) / rounds;

    phase('structural edits');
    // Structural edits: reorder one layer, insert one, delete one.
    const structural = {};
    for (const kind of ['reorder', 'insert', 'delete']) {
      applies.length = 0; flushes.length = 0;
      const n = Math.max(10, Math.floor(rounds / 5));
      for (let i = 0; i < n; i++) {
        PM.hist.do(kind, () => {
          const list = PM.proj.layers;
          if (kind === 'reorder') { const [moved] = list.splice(7, 1); list.splice(400, 0, moved); }
          else if (kind === 'insert') PM.addLayer(PM.mkLayer('solid', { name: `Bench ${i}` }), 300);
          else { const victim = list.findIndex(l => l.name?.startsWith('Bench ')); if (victim >= 0) list.splice(victim, 1); }
        });
        flush();
      }
      structural[kind] = { apply: stats(applies.slice()), flush: stats(flushes.slice()) };
    }
    phase('memory and consistency');
    wasm.apply = apply;
    const heap = wasm.heap();
    const check = doc.check('bench');
    return {
      layers: layers.length, channels: channels.length, animated: animated.length,
      json: JSON.stringify(PM.proj).length,
      loadMs,
      evP: { allCachedNs: evPAllNs, animatedCachedNs: evPAnimatedNs, animatedNewTimeNs: evPNewTimeNs, batchPerValueNs: batchNs, legacyAllCachedNs: legacyAllNs },
      property, propertyBackToBackMs, structural,
      memory: { heapLiveMB: heap.live / 1e6, heapPeakMB: heap.peak / 1e6, linearMB: heap.memory / 1e6 },
      consistency: { ok: check?.ok, ms: check?.ms },
      timerResolutionMs: (() => { let a = performance.now(), b; do { b = performance.now(); } while (b === a); return b - a; })(),
    };
  }, { project, rounds });
  console.log(JSON.stringify(result, null, 2));
} finally {
  clearTimeout(deadline);
  await quit();
  await rm(userData, { recursive: true, force: true });
}
