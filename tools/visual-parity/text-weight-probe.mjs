// Today's text weight, measured (TEXT-SMOOTH-1, pm-text `smoothing`).
//
//   node tools/visual-parity/text-weight-probe.mjs --out <dir>
//
// v1.2.0 (hidden, a fresh empty profile under <dir>) draws one "l" per case
// through its export path (`renderToPixels`, Full quality): Helvetica
// Regular and Bold, white on black and black on white, at device sizes 10
// to 96 px. Each case's stem coverage width (the sum across the stem's middle
// row of (L − background) / (paint − background), display values) goes to
// <dir>/v1-stems.json. The engine's `text_weight` test prints the same
// measurement for its own weights.
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

const argv = process.argv.slice(2);
const out = path.resolve(argv[argv.indexOf('--out') + 1]);
mkdirSync(out, { recursive: true });
const profile = path.join(out, 'v1-empty-profile');
rmSync(profile, { recursive: true, force: true });
mkdirSync(profile, { recursive: true });
const desktop = path.join(process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0', 'apps/desktop');
const req = createRequire(path.join(desktop, 'package.json'));
const { _electron } = req('playwright');
const env = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1' };
delete env.ELECTRON_RUN_AS_NODE;
const app = await _electron.launch({ executablePath: req('electron'), args: ['--use-mock-keychain', '.'], cwd: desktop, env, timeout: 600_000 });
try {
  const page = await app.firstWindow({ timeout: 600_000 });
  await page.waitForFunction(() => Boolean(window.PM?.app), null, { timeout: 600_000 });
  await page.evaluate(async () => { await window.PM.projectReady; await window.PM.Kernel.loader.builtinsReady; });
  const cases = [];
  for (const weight of [400, 700]) for (const [paint, bg] of [['#FFFFFF', '#000000'], ['#000000', '#FFFFFF']]) {
    for (const size of [10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 64, 96]) cases.push({ weight, paint, bg, size });
  }
  const results = await page.evaluate(async (cases) => {
    const PM = window.PM;
    const W = 256, H = 256;
    const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
    const lum = (r, g, b) => 0.2126 * r + 0.7152 * g + 0.0722 * b;
    const out = [];
    for (const c of cases) {
      const proj = PM.mkProject({ name: 'weight probe', w: W, h: H, dur: 1, fps: 30, bg: c.bg });
      PM.replaceProject(proj);
      const layer = PM.mkLayer('text', { name: 'l', d: { text: { v: 'l', kf: [], expr: null }, font: 'Helvetica', weight: c.weight, size: c.size, color: c.paint, align: 'center' } });
      layer.p['position.x'].v = W / 2; layer.p['position.y'].v = H / 2;
      PM.proj.layers = [layer];
      PM.touch?.(); PM.invalidate?.();
      await document.fonts?.ready;
      await PM.prepareFrame(0);
      let px = PM.GL.renderToPixels(0, W, H, { mblur: false, opaque: true, topDownOpaque: true });
      await new Promise((r) => setTimeout(r, 50));
      await PM.prepareFrame(0);
      px = PM.GL.renderToPixels(0, W, H, { mblur: false, opaque: true, topDownOpaque: true });
      const [fr, fg, fb] = hex(c.paint), [br, bg, bb] = hex(c.bg);
      const lf = lum(fr, fg, fb), lb = lum(br, bg, bb);
      const cov = (x, y) => { const i = (y * W + x) * 4; return (lum(px[i], px[i + 1], px[i + 2]) - lb) / (lf - lb); };
      const rows = [];
      for (let y = 0; y < H; y++) { let m = 0; for (let x = 0; x < W; x++) m = Math.max(m, cov(x, y)); if (m > 0.5) rows.push(y); }
      if (!rows.length) { out.push({ ...c, error: 'no ink' }); continue; }
      const y = rows[Math.floor(rows.length / 2)];
      let width = 0; for (let x = 0; x < W; x++) width += Math.min(1, Math.max(0, cov(x, y)));
      let height = 0; const xs = [...Array(W).keys()].filter((x) => cov(x, y) > 0.5); const xc = xs[Math.floor(xs.length / 2)];
      for (let yy = 0; yy < H; yy++) height += Math.min(1, Math.max(0, cov(xc, yy)));
      out.push({ ...c, width, height });
    }
    return out;
  }, cases);
  writeFileSync(path.join(out, 'v1-stems.json'), JSON.stringify(results, null, 1));
  for (const r of results) console.log(`${r.weight} ${r.paint} on ${r.bg} ${String(r.size).padStart(3)} px: width ${r.width?.toFixed(3)} height ${r.height?.toFixed(2)}${r.error ? ' ' + r.error : ''}`);
} finally {
  const pid = app.process().pid;
  await Promise.race([app.close().catch(() => {}), new Promise((r) => setTimeout(r, 15_000))]);
  try { process.kill(pid, 'SIGKILL'); } catch {}
}
