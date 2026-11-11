// v1 reference frames for the visual parity sheet (README.md). Node only: Bun
// cannot host Playwright's Electron driver (engine/corpus/src/electron-bridge.mts).
//
//   node tools/visual-parity/v1-frames.mjs --profile <v1 profile clone> --project <file.pmv>
//        --out <dir> [--times 0,12.333] [--step 2] [--width 1280]
//
// v1.2.0 runs hidden (POWERMOVE_BACKGROUND_TEST=1) on the given profile CLONE
// with a mock Keychain, opens a copy of the project and writes each sample as
// the exporter draws it (PM.GL.renderToPixels with the export options, Full
// quality, after prepareFrame and a settle loop) to <out>/v1/<time>.png, plus
// <out>/v1/manifest.json. Never pass a real profile: the clone is written to.
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { encodePng, parseArgs, sampleTimes, timeName } from './lib.mjs';

const args = parseArgs(process.argv.slice(2));
const v1Root = process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0';
const desktop = path.join(v1Root, 'apps/desktop');
const { _electron } = createRequire(path.join(desktop, 'package.json'))('playwright');
const electronBin = createRequire(path.join(desktop, 'package.json'))('electron');

const profile = path.resolve(args.profile);
const live = path.join(process.env.HOME, 'Library/Application Support');
if (profile.startsWith(live + path.sep)) throw new Error(`${profile} is a live profile; pass a clone`);
if (!existsSync(profile)) throw new Error(`${profile} does not exist`);
const out = path.resolve(args.out, 'v1');
mkdirSync(out, { recursive: true });
const copy = path.join(path.resolve(args.out), 'v1-' + path.basename(args.project));
copyFileSync(args.project, copy);

const env = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1' };
delete env.ELECTRON_RUN_AS_NODE;
const log = (line) => console.log(`[v1 ${new Date().toISOString().slice(11, 19)}] ${line}`);
log(`launch on ${profile}`);
const app = await _electron.launch({
  executablePath: electronBin, args: ['--use-mock-keychain', '.'], cwd: desktop, env, timeout: 600_000,
});
try {
  await app.evaluate(({ dialog }, file) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [file] });
    dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' });
    dialog.showMessageBox = async () => ({ response: 1, checkboxChecked: false });
  }, copy);
  const page = await app.firstWindow({ timeout: 600_000 });
  page.on('pageerror', (e) => log(`pageerror ${String(e).split('\n')[0]}`));
  await page.waitForFunction(() => Boolean(window.PM?.app), null, { timeout: 600_000 });
  await page.evaluate(async () => { await window.PM.projectReady; await window.PM.Kernel.loader.builtinsReady; });
  log('app ready; opening project');
  const before = await page.evaluate(() => window.PM.proj?.id);
  await page.evaluate(() => window.PM.openProject());
  await page.waitForFunction((id) => window.PM.proj?.id !== id, before, { timeout: 300_000 });
  await page.evaluate(async () => { await window.PM.projectAssetsReady; });
  const pending = await page.evaluate(waitForMedia, 180_000);
  if (pending.length) log(`media still pending: ${pending.join('; ')}`);
  const info = await page.evaluate(() => {
    const p = window.PM.proj;
    return { name: p.name, w: p.w, h: p.h, fps: p.fps, dur: p.dur };
  });
  log(`opened ${info.name} ${info.w}x${info.h} ${info.fps} fps ${info.dur}s`);
  // User extensions register after the built-ins: wait until the project's
  // effect definitions stop arriving (none missing, or no change for 10 s),
  // or the first frames are drawn with those effects bypassed.
  const missingNow = () => page.evaluate(() => {
    const out = new Set();
    for (const comp of [window.PM.proj, ...Object.values(window.PM.proj.comps || {})]) for (const layer of comp.layers) {
      for (const fx of layer.fx || []) if (!window.PM.FX?.[fx.type]) out.add(fx.type);
    }
    return [...out].sort();
  });
  let missing = await missingNow();
  for (let quiet = 0, waited = 0; missing.length && quiet < 10_000 && waited < 90_000; waited += 500) {
    await page.waitForTimeout(500);
    const next = await missingNow();
    quiet = next.join() === missing.join() ? quiet + 500 : 0;
    missing = next;
  }
  if (missing.length) log(`effects without a definition (bypassed): ${missing.join(', ')}`);
  info.missingEffects = missing;
  const times = sampleTimes(info, args);
  // --width renders smaller, as v1's viewer below Full and its agent stills do.
  const width = Number(args.width) || info.w, height = Math.round(width * info.h / info.w);
  const samples = [];
  for (const time of times) {
    const started = Date.now();
    const r = await page.evaluate(captureFrame, { time, width, height });
    const file = `${timeName(time)}.png`;
    writeFileSync(path.join(out, file), encodePng(width, height, Buffer.from(r.base64, 'base64')));
    samples.push({ time, file, stable: r.stable, renders: r.renders, prepareError: r.prepareError, ms: Date.now() - started });
    log(`${time.toFixed(3)}s ${r.stable ? 'stable' : 'UNSTABLE'} after ${r.renders} renders${r.prepareError ? ` prepareError ${r.prepareError}` : ''}`);
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ source: `v1.2.0 export path, ${width}x${height}`, project: info, pending, samples }, null, 1));
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  // v1's quit barrier can wait on the renderer forever; the clone needs no clean quit.
  const pid = app.process().pid;
  await Promise.race([app.close().catch(() => {}), new Promise((r) => setTimeout(r, 15_000))]);
  try { process.kill(pid, 'SIGKILL'); } catch {}
  process.exit(process.exitCode ?? 0);
}

async function waitForMedia(timeoutMs) {
  const PM = window.PM;
  const deadline = performance.now() + timeoutMs;
  const pending = () => {
    const out = [];
    if (PM.assets.loading?.size) out.push(`loading: ${[...PM.assets.loading].join(', ')}`);
    for (const [id, error] of PM.assets.errors ?? []) out.push(`error ${id}: ${error}`);
    for (const comp of [PM.proj, ...Object.values(PM.proj.comps || {})]) for (const layer of comp.layers) {
      if (layer.type !== 'video') continue;
      const asset = PM.assets.get?.(layer.d.asset);
      if (!asset) out.push(`video ${layer.name}: asset not loaded`);
      else if (asset.el && asset.el.readyState < 2) out.push(`video ${layer.name}: readyState ${asset.el.readyState}`);
    }
    return out;
  };
  let last = pending();
  while (last.some((item) => !item.startsWith('error')) && performance.now() < deadline) {
    await new Promise((r) => setTimeout(r, 100));
    last = pending();
  }
  await document.fonts?.ready;
  return last;
}

// The exporter's frame (core/frame-capture.ts renderOpaqueFrame), after the
// frame has settled: three identical quarter-size renders in a row (async
// shader compiles, model loads, fonts, extension rasters).
async function captureFrame({ time, width, height }) {
  const PM = window.PM;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const quality = PM.quality;
  PM.pause();
  PM.Export.busy = true;
  let prepareError, stable = false, renders = 0;
  try {
    PM.quality = 1;
    const w = Math.max(2, Math.round(width / 4)), h = Math.max(2, Math.round(height / 4));
    let previous = '', streak = 0;
    const deadline = performance.now() + 30000;
    while (performance.now() < deadline) {
      try { await PM.prepareFrame(time); prepareError = undefined; } catch (e) { prepareError = String(e?.message || e); }
      const px = PM.GL.renderToPixels(time, w, h, { opaque: true, mblur: false });
      PM.preparedVideoFrames = null;
      renders++;
      let hash = 2166136261;
      for (let i = 0; i < px.length; i++) hash = Math.imul(hash ^ px[i], 16777619);
      streak = String(hash) === previous ? streak + 1 : 0;
      previous = String(hash);
      if (streak >= 2) { stable = true; break; }
      await sleep(150);
    }
    try { await PM.prepareFrame(time); } catch (e) { prepareError = String(e?.message || e); }
    const px = PM.GL.renderToPixels(time, width, height, {
      mblur: true, mbSamples: 16, shutter: PM.proj.shutter || .5, opaque: true, topDownOpaque: true,
    });
    if (!px || px.length !== width * height * 4) throw new Error('no pixels');
    let binary = '';
    for (let i = 0; i < px.length; i += 0x8000) binary += String.fromCharCode.apply(null, px.subarray(i, i + 0x8000));
    return { base64: btoa(binary), stable, renders, prepareError };
  } finally {
    PM.preparedVideoFrames = null;
    PM.Export.busy = false;
    PM.quality = quality;
  }
}
