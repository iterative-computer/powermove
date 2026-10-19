/*
 * Level 1 SVG images with "Use New Engine" on (decision S1;
 * docs/engine/vector.md, "Level 1 SVG images"): Chromium draws today's
 * bucket rasters in the window (engine-connection/svg-rasters.ts) and the
 * engine composites them. The viewer's frames ask for them, a synchronous
 * snapshot draws them on the calling thread, and an export in the job
 * engine gets them through host services from this window. Every frame is
 * compared byte for byte with today's compositor (`PM.GL.renderToPixels`,
 * the corpus capture's opaque frame) drawing the same document in the same
 * window, and the exported still with today's exporter's still of the same
 * project.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-svg.spec.ts --workers=1
 */
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { inflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import type { Page } from 'playwright';

import { expect, test, launchApp, type LaunchedApp } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };

/** An 8-bit RGB or RGBA, non-interlaced PNG's pixels (as RGBA). */
function decodePng(file: Buffer): { width: number; height: number; data: Uint8Array } {
  let at = 8, width = 0, height = 0, channels = 4;
  const idat: Buffer[] = [];
  while (at < file.length) {
    const length = file.readUInt32BE(at), type = file.toString('latin1', at + 4, at + 8), body = file.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') {
      width = body.readUInt32BE(0); height = body.readUInt32BE(4);
      if (body[8] !== 8 || body[12] !== 0) throw new Error('only 8-bit, non-interlaced PNGs');
      channels = body[9] === 6 ? 4 : body[9] === 2 ? 3 : 0;
      if (!channels) throw new Error(`PNG color type ${body[9]}`);
    }
    if (type === 'IDAT') idat.push(body);
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(idat)), stride = width * channels;
  const rows = new Uint8Array(stride * height), data = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]!, src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? rows[y * stride + x - channels]! : 0, up = y ? rows[(y - 1) * stride + x]! : 0, ul = x >= channels && y ? rows[(y - 1) * stride + x - channels]! : 0;
      const p = left + up - ul, pa = Math.abs(p - left), pb = Math.abs(p - up), pc = Math.abs(p - ul);
      const predictor = [0, left, up, (left + up) >> 1, pa <= pb && pa <= pc ? left : pb <= pc ? up : ul][filter]!;
      rows[y * stride + x] = (src[x]! + predictor) & 255;
    }
  }
  for (let i = 0, j = 0; i < width * height; i++, j += channels) {
    data[i * 4] = rows[j]!; data[i * 4 + 1] = rows[j + 1]!; data[i * 4 + 2] = rows[j + 2]!; data[i * 4 + 3] = channels === 4 ? rows[j + 3]! : 255;
  }
  return { width, height, data };
}

/** A gradient, a clip, a stroke, a blur and an even-odd path: the paint
 *  features whose Chromium rasterization emulation missed. */
const ART = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4C8DFF"/><stop offset="1" stop-color="#E83A5E" stop-opacity=".6"/></linearGradient>
<clipPath id="c"><circle cx="120" cy="45" r="34"/></clipPath><filter id="b"><feGaussianBlur stdDeviation="2.5"/></filter></defs>
<rect x="4" y="4" width="152" height="82" rx="14" fill="url(#g)"/>
<g clip-path="url(#c)"><rect x="80" y="0" width="80" height="90" fill="#FFD23F"/><path d="M80 90 L160 0" stroke="#14161C" stroke-width="6"/></g>
<path fill-rule="evenodd" d="M20 20 h50 v50 h-50 z M32 32 h26 v26 h-26 z" fill="#FFFFFF" filter="url(#b)"/>
<path d="M10 80 Q 60 40 110 80" fill="none" stroke="#7CFFB2" stroke-width="3.5" stroke-dasharray="7 4" stroke-linecap="round"/>
</svg>`;

/** The same layers over a PNG of the SVG at its natural size instead of
 *  the SVG (the export test's control: no SVG raster involved). */
async function pngProject(page: Page): Promise<void> {
  await svgProject(page);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    const svg = Object.values(PM.proj.assets).find((a: any) => a.format === 'svg') as any;
    const canvas = document.createElement('canvas');
    canvas.width = 160; canvas.height = 90;
    canvas.getContext('2d')!.drawImage(PM.assets.get(svg.id).el, 0, 0, 160, 90);
    const blob: Blob = await new Promise((resolve) => canvas.toBlob((b) => resolve(b!), 'image/png'));
    const png = await PM.assets.add(new File([blob], 'art.png', { type: 'image/png' }), { newAsset: true, silent: true });
    for (const layer of PM.proj.layers) layer.d.asset = png.id;
    PM.touch();
  });
}

/** The project: three level 1 SVG image layers (contain at 100%; cover,
 *  rotated 23° at 230%; stretch with animated scale and opacity) over a
 *  solid background, 640 × 360. */
async function svgProject(page: Page): Promise<void> {
  await page.evaluate(async (svg) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'SVG compat', w: 640, h: 360, fps: 30, dur: 2, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    PM.replaceProject(project);
    const asset = await PM.assets.add(new File([svg], 'art.svg', { type: 'image/svg+xml' }), { newAsset: true, silent: true });
    const layer = (name: string, d: any, x: number, y: number) => {
      const l = PM.mkLayer('image', { name, from: 0, dur: 2, d: { asset: asset.id, ...d } }, PM.proj);
      l.p['position.x'].v = x; l.p['position.y'].v = y;
      return l;
    };
    const a = layer('Contain', { fit: 'contain', w: 200, h: 140 }, 130, 100);
    const b = layer('Cover', { fit: 'cover', w: 120, h: 120 }, 420, 120);
    b.p.rotation.v = 23; b.p['scale.x'].v = 230; b.p['scale.y'].v = 230;
    const c = layer('Stretch', { fit: 'stretch', w: 160, h: 60 }, 300, 290);
    c.p['scale.x'].kf = [PM.KF(0, 60), PM.KF(2, 310)];
    c.p.opacity.v = 70;
    PM.proj.layers = [b, c, a];
    PM.ProjectIndex?.invalidate?.();
    PM.touch();
    PM.Projects?.put?.(PM.proj);
  }, ART);
}

async function openEngine(session: LaunchedApp): Promise<void> {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender && (window as any).PM?.EngineSvgRasters));
  await svgProject(page);
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
}

/** Today's compositor and the engine for the same frame: bytes that differ. */
const compareFrame = (page: Page, time: number) => page.evaluate((t) => {
  const PM = (window as any).PM;
  const legacy: Uint8Array = PM.GL.renderToPixels(t, 640, 360, { opaque: true, topDownOpaque: true, mblur: false });
  const engine = PM.EngineRender.snapshotSync({ time: t, width: 640, height: 360, pixels: 'opaque', motionBlurSamples: 0, background: true });
  let differ = 0, max = 0, nonBackground = 0;
  for (let i = 0; i < legacy.length; i++) {
    const d = Math.abs(legacy[i]! - engine.data[i]!);
    if (d) differ++;
    if (d > max) max = d;
  }
  for (let i = 0; i < legacy.length; i += 4) if (legacy[i] !== 0x20 || legacy[i + 1] !== 0x30 || legacy[i + 2] !== 0x40) nonBackground++;
  return { differ, max, nonBackground, bytes: legacy.length, engineBytes: engine.data.length };
}, time);

test.describe('Use New Engine: level 1 SVG images', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('the viewer and snapshots draw Chromium’s rasters exactly as today’s compositor', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    // The viewer's frame asked for its buckets and the window answered.
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSvgRasters.stats.posted), { timeout: 20_000 }).toBeGreaterThanOrEqual(3);
    for (const t of [0, 0.75, 1.5]) {
      const r = await compareFrame(page, t);
      expect(r.engineBytes).toBe(r.bytes);
      expect(r.nonBackground).toBeGreaterThan(20_000);
      expect({ t, differ: r.differ, max: r.max }).toEqual({ t, differ: 0, max: 0 });
    }
    const stats = await page.evaluate(() => ({ ...(window as any).PM.EngineSvgRasters.stats }));
    console.log(`svg rasters: ${JSON.stringify(stats)}`);
    expect(stats.failed).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a replaced SVG is drawn from its new bytes, never the old raster', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    expect((await compareFrame(page, 0.5)).differ).toBe(0);
    // A point inside the clip circle, off the stroke: the clipped rect's
    // colour, in the contain layer.
    const at = (x: number, y: number) => page.evaluate(({ x, y }) => {
      const PM = (window as any).PM;
      const engine = PM.EngineRender.snapshotSync({ time: 0.5, width: 640, height: 360, pixels: 'opaque', motionBlurSamples: 0, background: true });
      const legacy: Uint8Array = PM.GL.renderToPixels(0.5, 640, 360, { opaque: true, topDownOpaque: true, mblur: false });
      const i = (y * 640 + x) * 4;
      return { engine: [...engine.data.slice(i, i + 3)], legacy: [...legacy.slice(i, i + 3)] };
    }, { x, y });
    const before = await at(205, 81);
    expect(before.engine).toEqual(before.legacy);
    await page.evaluate(async (svg) => {
      const PM = (window as any).PM;
      const asset = Object.values(PM.proj.assets).find((a: any) => a.format === 'svg') as any;
      await PM.assets.replace(asset.id, new File([svg.replace('#FFD23F', '#00C2A8')], 'art.svg', { type: 'image/svg+xml' }));
    }, ART);
    // The engine draws the new bytes (another content, asked for again),
    // as today's compositor does.
    await expect.poll(async () => (await at(205, 81)).engine, { timeout: 20_000 }).not.toEqual(before.engine);
    const after = await at(205, 81);
    console.log(`replace: before ${JSON.stringify(before)} after ${JSON.stringify(after)}`);
    expect(after.engine).toEqual([0x00, 0xC2, 0xA8]);
    expect(after.engine).toEqual(after.legacy);
    const replaced = await compareFrame(page, 0.5);
    expect({ differ: replaced.differ, max: replaced.max }).toEqual({ differ: 0, max: 0 });
    // Undo draws the old bytes again, and redo the new ones.
    await page.evaluate(() => (window as any).PM.hist.undo());
    await expect.poll(async () => (await at(205, 81)).engine, { timeout: 20_000 }).toEqual(before.engine);
    expect((await at(205, 81)).legacy).toEqual(before.engine);
    await page.evaluate(() => (window as any).PM.hist.redo());
    await expect.poll(async () => (await at(205, 81)).engine, { timeout: 20_000 }).toEqual(after.engine);
    expect((await compareFrame(page, 0.5)).differ).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a bucket change in the viewer shows a stand-in until the window answers', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSvgRasters.stats.posted), { timeout: 20_000 }).toBeGreaterThanOrEqual(3);
    const runs = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const presentation = PM.Kernel.services.get('viewer').enginePresentation;
      const contain = PM.proj.layers.find((l: any) => l.name === 'Contain');
      const out: { standIns: number; ms: number }[] = [];
      for (const scale of [131, 173, 219, 97, 151, 263, 117, 199, 241, 89]) {
        const posted = PM.EngineSvgRasters.stats.posted;
        let standIns = 0, done = 0;
        const start = performance.now();
        const off = presentation.onPresented((frame: any) => {
          if (PM.EngineSvgRasters.stats.posted === posted && !frame.standIn) return;
          if (frame.standIn) standIns++;
          else if (!done) done = performance.now();
        });
        contain.p['scale.x'].v = scale; contain.p['scale.y'].v = scale;
        PM.touch();
        PM.invalidate?.('render');
        const until = performance.now() + 5000;
        while (!done && performance.now() < until) await new Promise((r) => requestAnimationFrame(r));
        off();
        out.push({ standIns, ms: done ? Math.round(done - start) : -1 });
      }
      return out;
    });
    console.log(`bucket changes: ${JSON.stringify(runs)}`);
    // Every change ended on an exact frame after at most one stand-in
    // (the previous bucket, scaled), as the plan allows (vector.md).
    expect(runs.every((r) => r.ms >= 0)).toBe(true);
    expect(Math.max(...runs.map((r) => r.standIns))).toBeLessThanOrEqual(1);
    expect((await compareFrame(page, 0.5)).differ).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a restarted engine starts with no rasters and asks the window for them again', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSvgRasters.stats.posted), { timeout: 20_000 }).toBeGreaterThanOrEqual(3);
    const before = await page.evaluate(() => ({ connections: (window as any).PM.EngineConnection.status().connections, posted: (window as any).PM.EngineSvgRasters.stats.posted }));
    const pid = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid as number);
    expect(pid).toBeGreaterThan(0);
    process.kill(pid, 'SIGKILL');
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().connections), { timeout: 45_000 }).toBeGreaterThan(before.connections);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    // The new engine's viewer frame asked for the same buckets again.
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineSvgRasters.stats.posted), { timeout: 20_000 }).toBeGreaterThanOrEqual(before.posted + 3);
    for (const t of [0, 1.5]) {
      const r = await compareFrame(page, t);
      expect({ t, differ: r.differ, max: r.max }).toEqual({ t, differ: 0, max: 0 });
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

test('an export in the job engine gets the rasters from the window: the still equals today’s exporter’s', async () => {
  // PM_E2E_EVIDENCE keeps the stills there (outside the repository).
  const keep = process.env.PM_E2E_EVIDENCE;
  const root = keep ?? await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-svg-'));
  const exportStills = async (env: Record<string, string>, tag: string) => {
    const session = await launchApp({ env });
    try {
      await session.openEditor();
      if (env.POWERMOVE_NEW_ENGINE) {
        await expect.poll(() => session.app.evaluate(() => {
          const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
          return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
        }), { timeout: 30_000 }).toBe('ready/ready');
      }
      const out: Record<string, { file: string; produced: number | null }> = {};
      for (const [name, make] of [['svg', svgProject], ['png', pngProject]] as const) {
        await make(session.page);
        const file = path.join(root, `${tag}-${name}.png`);
        await session.app.evaluate(({ dialog }, filePath) => {
          dialog.showSaveDialog = async () => ({ canceled: false, filePath });
        }, file);
        const result = await session.page.evaluate(async () => {
          const PM = (window as any).PM;
          PM.setTime(1, { raw: true, force: true });
          const before = PM.EngineSvgRasters?.stats.produced ?? 0;
          const out = await PM.Export.run({ format: 'still', scale: 1, fps: 30, range: 'all', quality: 'high', mblur: false, audio: false });
          return { out, produced: PM.EngineSvgRasters ? PM.EngineSvgRasters.stats.produced - before : null };
        });
        expect(result.out).toEqual({ cancelled: false });
        out[name] = { file, produced: result.produced };
      }
      expect(session.diagnostics.pageErrors).toEqual([]);
      return out;
    } finally {
      await session.close();
    }
  };
  const compare = async (a: string, b: string) => {
    const [pa, pb] = [decodePng(await readFile(a)), decodePng(await readFile(b))];
    expect([pb.width, pb.height]).toEqual([pa.width, pa.height]);
    let differ = 0, max = 0;
    for (let i = 0; i < pa.data.length; i++) {
      const d = Math.abs(pa.data[i]! - pb.data[i]!);
      if (d) differ++;
      if (d > max) max = d;
    }
    return { differ, max };
  };
  try {
    const today = await exportStills({}, 'today');
    const engine = await exportStills(ON, 'engine');
    // The job engine asked this window for the SVG buckets (host services),
    // and for nothing in the PNG control.
    expect(engine.svg!.produced).toBeGreaterThanOrEqual(3);
    expect(engine.png!.produced).toBe(0);
    const svg = await compare(today.svg!.file, engine.svg!.file);
    const control = await compare(today.png!.file, engine.png!.file);
    console.log(`still export against today's: svg ${JSON.stringify(svg)}, png control ${JSON.stringify(control)}`);
    // Both must equal today's stills. The PNG control involves no SVG
    // raster: when it differs too, the difference is the job engine's
    // still pipeline, not the rasters (docs/engine/vector.md, "Level 1
    // measured").
    expect.soft({ control }).toEqual({ control: { differ: 0, max: 0 } });
    expect({ svg }).toEqual({ svg: { differ: 0, max: 0 } });
  } finally {
    if (!keep) await rm(root, { recursive: true, force: true });
  }
});
