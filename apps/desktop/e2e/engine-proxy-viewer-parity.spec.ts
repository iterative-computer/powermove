/** Video viewer comparisons distinguish source sampling from the editing copy. */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { inflateSync } from 'node:zlib';
import { test, expect, launchApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

type Bitmap = { data: Uint8Array; width: number; height: number; profile: string };
const colorProfileArgs = process.env.PM_PHASE4A_FORCE_SRGB === '1' ? ['--force-color-profile=srgb'] : [];

/** Read encoded pixels in the PNG's own profile. Electron's nativeImage
 * converts P3 screenshots to sRGB, amplifying a one-code P3 rounding delta
 * near an sRGB gamut boundary into seven codes. */
function decodePng(png: Buffer): Bitmap {
  expect(png.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))).toBe(true);
  let width = 0, height = 0, channels = 0, profile = '';
  const idat: Buffer[] = [];
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    const type = png.toString('ascii', offset + 4, offset + 8);
    const chunk = png.subarray(offset + 8, offset + 8 + length);
    if (type === 'IHDR') {
      width = chunk.readUInt32BE(0); height = chunk.readUInt32BE(4);
      expect(chunk[8]).toBe(8); // Eight-bit screenshots, no interlace.
      expect(chunk[12]).toBe(0);
      channels = chunk[9] === 2 ? 3 : chunk[9] === 6 ? 4 : 0;
      expect(channels).toBeGreaterThan(0);
    } else if (type === 'iCCP') {
      const terminator = chunk.indexOf(0);
      expect(chunk[terminator + 1]).toBe(0);
      profile += `iCCP:${createHash('sha256').update(inflateSync(chunk.subarray(terminator + 2))).digest('hex')};`;
    } else if (type === 'sRGB' || type === 'cICP' || type === 'gAMA' || type === 'cHRM') {
      profile += `${type}:${chunk.toString('hex')};`;
    } else if (type === 'IDAT') idat.push(chunk);
    offset += length + 12;
    if (type === 'IEND') break;
  }
  const packed = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const data = new Uint8Array(width * height * 4);
  let previous = new Uint8Array(stride), offset = 0;
  for (let y = 0; y < height; y++) {
    const filter = packed[offset++]!;
    expect(filter).toBeGreaterThanOrEqual(0);
    expect(filter).toBeLessThanOrEqual(4);
    const row = new Uint8Array(stride);
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? row[x - channels]! : 0;
      const above = previous[x]!;
      const upperLeft = x >= channels ? previous[x - channels]! : 0;
      const paeth = left + above - upperLeft;
      const predictor = Math.abs(paeth - left) <= Math.abs(paeth - above)
        && Math.abs(paeth - left) <= Math.abs(paeth - upperLeft) ? left
        : Math.abs(paeth - above) <= Math.abs(paeth - upperLeft) ? above : upperLeft;
      const adjustment = filter === 0 ? 0 : filter === 1 ? left : filter === 2 ? above
        : filter === 3 ? Math.floor((left + above) / 2) : predictor;
      row[x] = (packed[offset++]! + adjustment) & 255;
    }
    for (let x = 0; x < width; x++) {
      const src = x * channels, dst = (y * width + x) * 4;
      data[dst] = row[src]!; data[dst + 1] = row[src + 1]!;
      data[dst + 2] = row[src + 2]!; data[dst + 3] = channels === 4 ? row[src + 3]! : 255;
    }
    previous = row;
  }
  expect(offset).toBe(packed.length);
  return { data, width, height, profile };
}

async function captureBitmap(session: Awaited<ReturnType<typeof launchApp>>, evidence: string | undefined, name: string): Promise<Bitmap> {
  const clip = await session.page.locator('#gl').boundingBox();
  expect(clip).not.toBeNull();
  const png = await session.page.screenshot({ clip: clip!, animations: 'disabled' });
  if (evidence) { await mkdir(evidence, { recursive: true }); await writeFile(path.join(evidence, `${name}.png`), png); }
  return decodePng(png);
}

function compare(a: Bitmap, b: Bitmap) {
  expect([b.width, b.height]).toEqual([a.width, a.height]);
  expect(b.profile).toBe(a.profile);
  let max = 0, overOne = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const diff = Math.max(...[0, 1, 2].map((c) => Math.abs(a.data[i + c]! - b.data[i + c]!)));
    max = Math.max(max, diff); if (diff > 1) overOne++;
  }
  return { max, overOne, pixels: a.width * a.height };
}

function assertViewerParity(metrics: ReturnType<typeof compare>) {
  expect(metrics.max).toBeLessThanOrEqual(4);
  expect(metrics.overOne / metrics.pixels).toBeLessThanOrEqual(.001);
}

async function importVideo(session: Awaited<ReturnType<typeof launchApp>>, source: string, quality: number, resolution: string) {
  const { page } = session;
  await session.openEditor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.proj = PM.mkProject({ name: 'Half scale parity', w: 640, h: 360, fps: 24, dur: 1 });
    PM.touch(); PM.bus.emit('project');
    const input = document.createElement('input'); input.type = 'file'; input.id = 'half-source'; document.body.appendChild(input);
  });
  await page.locator('#half-source').setInputFiles(source);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.importFiles([(document.querySelector('#half-source') as HTMLInputElement).files![0]]);
    PM.sel.layers = []; PM.perf.auto = false;
    PM.pause(); PM.setTime(0, { raw: true, force: true }); PM.touch();
  });
  await page.waitForFunction(() => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((l: any) => l.type === 'video');
    return !!PM.assets.get(layer.d.asset)?.preview?.el;
  }, undefined, { timeout: 90_000 }).catch((error) => {
    throw new Error(`${String(error)}\n${JSON.stringify(session.diagnostics)}`);
  });
  await setView(session, quality, resolution);
}

async function setView(session: Awaited<ReturnType<typeof launchApp>>, quality: number, resolution: string) {
  await session.page.evaluate(({ quality, resolution }) => {
    const PM = (window as any).PM;
    PM.previewResolution = resolution; PM.quality = quality;
    PM.pause(); PM.setTime(0, { raw: true, force: true }); PM.touch(); PM.bus.emit('quality');
    const viewer = PM.Kernel.services.get('viewer'); viewer.setZoom(.5); viewer.layout(); PM.invalidate();
  }, { quality, resolution });
  await session.page.waitForTimeout(500);
}

/** Suppresses only today's disposable editing derivative for the source control. */
async function useOriginal(session: Awaited<ReturnType<typeof launchApp>>) {
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((l: any) => l.type === 'video');
    const asset = PM.assets.get(layer.d.asset);
    if (!asset?.preview?.el) throw new Error('Editing derivative was not ready');
    asset.preview = undefined;
    // A real derivative/source change emits `assets`, which clears the
    // legacy GL preview-frame cache. Without it this synthetic switch can
    // capture the prior editing FBO while reporting original selected.
    PM.bus.emit('assets');
    PM.setTime(0, { raw: true, force: true }); PM.invalidate('render');
  });
  await session.page.waitForTimeout(500);
}

async function videoSelection(session: Awaited<ReturnType<typeof launchApp>>) {
  return session.page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((l: any) => l.type === 'video');
    const asset = PM.assets.get(layer.d.asset);
    const editing = asset.preview?.el;
    return {
      selectedEditing: !!editing && (PM.perf?.auto || PM.quality < 1)
        && !PM.Export?.busy && !PM.agentFrameCapture && !PM.Preview?.preparing,
      sourceSize: [asset.el?.videoWidth, asset.el?.videoHeight],
      editingSize: editing ? [editing.videoWidth, editing.videoHeight] : null,
      quality: PM.quality,
      previewResolution: PM.previewResolution,
    };
  });
}

/** The fixture loops one image: any presented timestamp contains identical pixels. */
function assertStaticFrames(source: string) {
  const lines = execFileSync(ffmpegBinary, ['-v', 'error', '-i', source, '-f', 'framemd5', '-'], { encoding: 'utf8', timeout: 30_000 })
    .split('\n').filter(line => /^0,\s*\d+,\s*\d+,/.test(line));
  expect(lines).toHaveLength(24);
  expect(new Set(lines.map(line => line.split(',').at(-1)!.trim())).size).toBe(1);
}

test('video at Half viewer resolution matches the current editing preview', async () => {
  test.setTimeout(240_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-half-video-'));
  const evidence = process.env.PM_PHASE4A_EVIDENCE;
  const frames: Bitmap[] = [];
  let currentOriginal: Bitmap | undefined;
  let editingSelection: Awaited<ReturnType<typeof videoSelection>> | undefined;
  let originalSelection: Awaited<ReturnType<typeof videoSelection>> | undefined;
  try {
    const source = path.join(root, 'pattern.mov');
    execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=1920x1080:rate=24',
      '-vf', 'loop=loop=-1:size=1:start=0', '-frames:v', '24', '-c:v', 'prores_videotoolbox', '-profile:v', '2', '-pix_fmt', 'yuv422p10le', source], { timeout: 60_000 });
    assertStaticFrames(source);
    for (const native of [false, true]) {
      let session: Awaited<ReturnType<typeof launchApp>> | null = null;
      try {
        session = await launchApp({ userData: path.join(root, native ? 'native' : 'current'), env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' }, args: colorProfileArgs });
        await importVideo(session, source, .5, '0.5');
        if (native) await session.page.waitForFunction(() => (window as any).PM.EngineConnection?.session()?.lastFrame?.quality?.resolution === 'half', undefined, { timeout: 30_000 });
        if (!native) editingSelection = await videoSelection(session);
        frames.push(await captureBitmap(session, evidence, native ? 'native-half' : 'current-half'));
        if (!native) {
          await useOriginal(session);
          originalSelection = await videoSelection(session);
          currentOriginal = await captureBitmap(session, evidence, 'current-original-half');
        }
      } finally { await session?.close(); }
    }
    const metrics = compare(frames[0]!, frames[1]!);
    const originalControl = compare(currentOriginal!, frames[1]!);
    const originalSwitch = compare(frames[0]!, currentOriginal!);
    if (evidence) await writeFile(path.join(evidence, 'half-video-metrics.json'), JSON.stringify({ editingCopy: metrics, originalControl, originalSwitch, editingSelection, originalSelection }));
    expect(editingSelection?.selectedEditing).toBe(true);
    expect(originalSelection?.selectedEditing).toBe(false);
    expect(editingSelection?.sourceSize).not.toEqual(editingSelection?.editingSize);
    expect(originalSwitch.max).toBeGreaterThan(1);
    // The original control diagnoses the stage at which pixels diverge. It
    // cannot replace the editing-copy baseline that existing projects show.
    assertViewerParity(metrics);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('Auto, Half and Quarter sample the full video source at the viewer canvas scale', async () => {
  test.setTimeout(300_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-original-video-'));
  const evidence = process.env.PM_PHASE4A_EVIDENCE;
  const views = [
    { name: 'auto', resolution: 'auto', quality: 1, engineResolution: 'full' },
    { name: 'half', resolution: '0.5', quality: .5, engineResolution: 'half' },
    { name: 'quarter', resolution: '0.25', quality: .25, engineResolution: 'quarter' },
  ] as const;
  const captures = new Map<string, Bitmap>();
  try {
    // VP9 is native to both Chromium and pm-media. Neither viewer needs a
    // playback conversion, and the derivative is removed after it is ready.
    const source = path.join(root, 'pattern.webm');
    execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=1920x1080:rate=24',
      '-vf', 'loop=loop=-1:size=1:start=0', '-frames:v', '24', '-c:v', 'libvpx-vp9',
      '-pix_fmt', 'yuv420p', '-b:v', '0', '-crf', '18', '-g', '1', '-threads', '4', source], { timeout: 60_000 });
    assertStaticFrames(source);
    for (const native of [false, true]) {
      let session: Awaited<ReturnType<typeof launchApp>> | null = null;
      try {
        session = await launchApp({ userData: path.join(root, native ? 'native' : 'current'),
          env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' }, args: colorProfileArgs });
        await importVideo(session, source, 1, 'auto');
        await useOriginal(session);
        for (const view of views) {
          await setView(session, view.quality, view.resolution);
          if (native) await session.page.waitForFunction((expected) =>
            (window as any).PM.EngineConnection?.session()?.lastFrame?.quality?.resolution === expected,
          view.engineResolution, { timeout: 30_000 });
          captures.set(`${native ? 'native' : 'current'}-${view.name}`,
            await captureBitmap(session, evidence, `source-${native ? 'native' : 'current'}-${view.name}`));
        }
      } finally { await session?.close(); }
    }
    const metrics = Object.fromEntries(views.map(view => [view.name,
      compare(captures.get(`current-${view.name}`)!, captures.get(`native-${view.name}`)!)]));
    if (evidence) await writeFile(path.join(evidence, 'source-video-metrics.json'), JSON.stringify(metrics));
    for (const view of views) assertViewerParity(metrics[view.name]!);
  } finally { await rm(root, { recursive: true, force: true }); }
});
