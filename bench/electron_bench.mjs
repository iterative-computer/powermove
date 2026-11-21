// Powermove v1 (Electron) benchmark driver. Drives the packaged app with Playwright's Electron
// launcher (the same mechanism as tools/perf-gate/electron.mjs), with an isolated profile.
//   node electron_bench.mjs build <appBinary> <mediaDir> <outDir>          -> writes A/B/C .pmv
//   node electron_bench.mjs run <appBinary> <scene.pmv> <outDir> <run#> [--export]
// Emits one JSON object per measurement on stdout ("BENCH {...}").
import path from 'node:path';
import { mkdtemp, writeFile, readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
const require = createRequire(process.env.PM_ELECTRON_PKG || path.join(process.env.HOME, 'bench-work/electron/apps/desktop/package.json'));
const { _electron } = require('playwright');
const HERE = path.dirname(new URL(import.meta.url).pathname);
const emit = v => process.stdout.write('BENCH ' + JSON.stringify(v) + '\n');
delete process.env.ELECTRON_RUN_AS_NODE;

async function launch(bin, template = null) {
  const profile = await mkdtemp('/tmp/pm-bench-electron-');
  if (template) execFileSync('ditto', [template, profile]);
  await writeFile(path.join(profile, 'onboarding-v1.json'), '{"version":1}\n', { mode: 0o600 });
  const t0 = Date.now();
  const app = await _electron.launch({ executablePath: bin, args: [], timeout: 60_000,
    env: { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '0', POWERMOVE_DISABLE_RUNTIME_UPDATES: '1' } });
  const page = await app.firstWindow();
  page.setDefaultTimeout(120_000);
  await page.waitForFunction(() => Boolean(window.PM?.app));
  await page.evaluate(async () => { await window.PM.projectReady; await window.PM.Kernel?.loader?.builtinsReady; });
  // Usable: the editor window is visible and its renderer has painted a frame.
  await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
  const visible = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().some(w => w.isVisible()));
  const launchS = (Date.now() - t0) / 1000;
  // Save/export destinations: answer the native save dialog with a fixed path (harness-only stub).
  await app.evaluate(({ dialog }) => { dialog.showSaveDialog = async () => ({ canceled: !globalThis.__benchSave, filePath: globalThis.__benchSave }); });
  return { app, page, profile, launchS, visible };
}
const setSavePath = (app, p) => app.evaluate((_, p) => { globalThis.__benchSave = p; }, p);

function family(app) {
  return app.evaluate(({ app }) => app.getAppMetrics().map(m => m.pid));
}
function rssMB(pids) {
  try {
    const out = execFileSync('ps', ['-o', 'rss=', '-p', pids.join(',')], { encoding: 'utf8' });
    return out.split('\n').filter(Boolean).reduce((a, b) => a + Number(b), 0) / 1024;
  } catch { return null; }
}
async function sampler(app) {
  let peak = 0, stop = false; const samples = [];
  const loop = (async () => { while (!stop) { const v = rssMB(await family(app)); if (v) { peak = Math.max(peak, v); samples.push(v); } await new Promise(r => setTimeout(r, 200)); } })();
  return async () => { stop = true; await loop; return { peak_mb: peak, samples: samples.length }; };
}

async function build(bin, media, out) {
  const { app, page, profile } = await launch(bin);
  await page.evaluate(await readFile(path.join(HERE, 'scenes.js'), 'utf8'));
  for (const name of ['A', 'B', 'C']) {
    const doc = await page.evaluate(n => JSON.parse(JSON.stringify(window.__benchScenes['scene' + n]())), name);
    await page.evaluate(async doc => {
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: doc }));
      PM.ProjectsScreen?.hide?.();
      for (let i = 0; i < 400 && PM.proj?.id !== doc.id; i++) await new Promise(r => setTimeout(r, 25));
      if (PM.proj?.id !== doc.id) throw new Error('did not open');
      await PM.projectAssetsReady;
    }, doc);
    if (name === 'B') {
      await page.evaluate(() => { const i = document.createElement('input'); i.type = 'file'; i.multiple = true; i.id = 'bench-media'; i.style.display = 'none'; document.body.appendChild(i); });
      const files = ['clip1.mp4', 'clip2.mp4', 'clip3.mp4', 'clip4.mp4', 'clip5.mp4', 'clip6.mp4', 'music.wav'].map(f => path.join(media, f));
      await page.setInputFiles('#bench-media', files);
      const rewritten = await page.evaluate(async () => {
        const map = {};
        for (const f of Array.from(document.getElementById('bench-media').files)) map[f.name] = (await PM.assets.add(f, { newAsset: true, silent: true })).id;
        const visit = v => typeof v === 'string' && v.startsWith('media:') ? (map[v.slice(6)] ?? v) : Array.isArray(v) ? v.map(visit)
          : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, visit(x)])) : v;
        const p = JSON.parse(JSON.stringify(PM.proj)); p.layers = visit(p.layers); p.comps = visit(p.comps);
        return p;
      });
      await page.evaluate(async doc => {
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: doc }));
        for (let i = 0; i < 400 && !(PM.proj?.id === doc.id && PM.proj.layers.every(l => !String(l.d?.asset ?? '').startsWith('media:'))); i++) await new Promise(r => setTimeout(r, 25));
        await PM.projectAssetsReady;
      }, rewritten);
    }
    await page.evaluate(() => PM.Projects.put(PM.proj));
    const file = path.join(out, `scene-${name}.pmv`);
    await setSavePath(app, file);
    const res = await page.evaluate(async () => { const r = await PM.Export.run({ format: 'json', reportPath: true }); return { path: r?.path, error: r?.error ?? null, layers: PM.proj.layers.length, comps: Object.keys(PM.proj.comps || {}).length }; });
    emit({ op: 'build', scene: name, ...res });
  }
  const names = await page.evaluate(async () => { await new Promise(r => setTimeout(r, 1500)); return PM.Projects.list().map(m => m.name); });
  for (const n of ['Bench A motion graphics', 'Bench B edit', 'Bench C effects stress'])
    if (!names.includes(n)) throw new Error('scene not persisted in library: ' + n);
  await quit(app);
  // The scenes stay in this profile's project library; runs start from a copy of it (see openScene).
  const template = path.join(out, 'electron-profile-template');
  execFileSync('rm', ['-rf', template]); execFileSync('ditto', [profile, template]);
  for (const junk of ['SingletonCookie', 'SingletonLock', 'SingletonSocket', 'DevToolsActivePort', 'Cache', 'Code Cache', 'GPUCache', 'DawnGraphiteCache', 'DawnWebGPUCache'])
    execFileSync('rm', ['-rf', path.join(template, junk)]);
  emit({ op: 'template', path: template });
}
async function quit(app) {
  const done = new Promise(r => app.process().once('exit', r));
  await app.evaluate(({ app }) => setTimeout(() => app.exit(0), 10)).catch(() => {});
  await Promise.race([done, new Promise(r => setTimeout(r, 15000))]);
  try { app.process().kill('SIGKILL'); } catch {}
}

async function openScene(app, page, file) {
  // Opening a .pmv from disk fails in this production build (both the macOS open-file and the renderer file
  // paths; see RESULTS.md "Bugs"). The run therefore opens the same project from the app's own project
  // library (Recents), in a fresh copy of the profile the scenes were built and saved in.
  const name = /"name":"(Bench [^"]+)"/.exec((await readFile(file)).subarray(0, 1 << 20).toString('latin1'))[1];
  // Start every open from a fresh untitled project, so the target is never the session-restored one.
  await page.evaluate(async () => {
    const doc = PM.mkProject({ name: 'Bench blank', w: 1920, h: 1080, fps: 30, dur: 10 });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: doc }));
    for (let i = 0; i < 400 && PM.proj?.id !== doc.id; i++) await new Promise(r => setTimeout(r, 25));
    if (PM.proj?.id !== doc.id) throw new Error('blank project did not open');
    PM.ProjectsScreen?.hide?.(); await new Promise(r => setTimeout(r, 1000));
  });
  const r = await page.evaluate(name => new Promise((resolve, reject) => {
    const meta = PM.Projects.list().find(m => m.name === name);
    if (!meta) return reject(new Error('not in library: ' + name));
    const before = PM.proj?.id;
    const to = setTimeout(() => reject(new Error('open timed out')), 120000);
    const poll = setInterval(() => { if (PM.proj && PM.proj.id !== before && PM.ProjectsScreen?.isOpen) { PM.ProjectsScreen.hide(); PM.invalidate?.(); } }, 10);
    const off = PM.bus.on('preview:presented', () => {
      if (!PM.proj || PM.proj.id !== meta.id || PM.ProjectsScreen?.isOpen) return;
      off(); clearTimeout(to); clearInterval(poll);
      requestAnimationFrame(() => resolve({ ms: performance.now() - t0, name: PM.proj.name }));
    });
    const t0 = performance.now();
    PM.openProjectHere(meta.id);
  }), name);
  await page.evaluate(async () => { await PM.projectAssetsReady; });
  return { open_s: r.ms / 1000, project: r.name, endpoint: 'PM.openProjectHere -> first preview:presented + rAF' };
}

async function playback(page, seconds = 10) {
  return page.evaluate(async seconds => {
    PM.pause(); PM.setTime(0); PM.invalidate?.();
    await new Promise(r => setTimeout(r, 1000));
    const times = new Set(); let events = 0;
    const off = PM.bus.on('preview:presented', e => { events++; times.add(Math.round(Number(e.time) * PM.proj.fps)); });
    const t0 = performance.now(); PM.play();
    await new Promise(r => setTimeout(r, seconds * 1000 - 50));
    PM.pause(); const el = (performance.now() - t0) / 1000; off();
    const expected = Math.round(el * PM.proj.fps);
    return { elapsed_s: el, presented_events: events, distinct_frames: times.size, fps: times.size / el, dropped: Math.max(0, expected - times.size), expected, viewer: [PM.Kernel.services.get('viewer')?.stage?.clientWidth, PM.Kernel.services.get('viewer')?.stage?.clientHeight] };
  }, seconds);
}

async function scrub(page, n = 60, seed = 12345) {
  return page.evaluate(async ({ n, seed }) => {
    let s = seed; const rnd = () => (s = (s * 1103515245 + 12345) % 2147483648) / 2147483648;
    const fps = PM.proj.fps, frames = Math.round(PM.proj.dur * fps), out = [];
    PM.pause();
    for (let i = 0; i < n; i++) {
      const target = Math.floor(rnd() * frames) / fps;
      const t0 = performance.now();
      await new Promise((resolve, reject) => {
        const to = setTimeout(() => reject(new Error('seek timeout')), 20000);
        const off = PM.bus.on('preview:presented', e => {
          if (Math.abs(Number(e.time) - target) > 1e-5) return;
          off(); clearTimeout(to);
          const gl = PM.GL.gl, sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0); gl.flush();
          const poll = () => { const st = gl.clientWaitSync(sync, 0, 0); if (st === gl.TIMEOUT_EXPIRED) return setTimeout(poll, 1); gl.deleteSync(sync); requestAnimationFrame(() => resolve()); };
          poll();
        });
        PM.setTime(target); PM.invalidate?.();
      });
      out.push(performance.now() - t0);
    }
    return out;
  }, { n, seed });
}

async function exportVideo(app, page, format, file) {
  await setSavePath(app, file);
  const t0 = Date.now();
  const r = await page.evaluate(async format => {
    const o = Object.assign({}, PM.Export.defaults(), { name: PM.proj.name, format, range: 'all', audio: true, fps: PM.proj.fps, scale: 1, reportPath: true });
    const res = await PM.Export.run(o);
    return { path: res?.path ?? null, error: res?.error ?? null, stats: PM.Export.lastStats ?? null, quality: o.quality };
  }, format);
  return { export_s: (Date.now() - t0) / 1000, ...r };
}

async function run(bin, scene, out, runIndex, doExport, template) {
  const L = await launch(bin, template);
  const { app, page } = L;
  emit({ op: 'launch', run: runIndex, launch_s: L.launchS, visible: L.visible });
  const o = await openScene(app, page, scene);
  emit({ op: 'open', run: runIndex, scene, ...o });
  await new Promise(r => setTimeout(r, 3000));
  emit({ op: 'idle_rss', run: runIndex, scene, rss_mb: rssMB(await family(app)) });
  const stopP = await sampler(app);
  const pb = await playback(page);
  emit({ op: 'playback', run: runIndex, scene, ...pb, mem: await stopP() });
  const s = await scrub(page);
  const sorted = [...s].sort((a, b) => a - b);
  emit({ op: 'scrub', run: runIndex, scene, median_ms: sorted[30], p95_ms: sorted[Math.ceil(.95 * 60) - 1], samples_ms: s });
  if (doExport) for (const fmt of ['mp4', 'prores']) {
    const file = path.join(out, `${path.basename(scene, '.pmv')}-electron-r${runIndex}.${fmt === 'mp4' ? 'mp4' : 'mov'}`);
    const stopE = await sampler(app);
    const e = await exportVideo(app, page, fmt, file);
    emit({ op: 'export', run: runIndex, scene, format: fmt === 'mp4' ? 'mp4' : 'prores4444', file, ...e, mem: await stopE() });
  }
  await quit(app);
  execFileSync('rm', ['-rf', L.profile]);
}

const [cmd, ...a] = process.argv.slice(2);
if (cmd === 'build') await build(a[0], a[1], a[2]);
else if (cmd === 'run') await run(a[0], a[1], a[2], Number(a[3]), a.includes('--export'),
  a.find(x => x.startsWith('--template='))?.slice(11) ?? path.join(path.dirname(a[1]), 'electron-profile-template'));
else throw new Error('usage');
