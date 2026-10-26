import { requestJobEngine } from './helpers/job-engine';
import { execFileSync } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test, type ElectronApplication, type Page } from '@playwright/test';

import { launchApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';
import { compareExports } from './helpers/export-encoding-parity';
import { avFoundationImport, proResFrameHeaderCodes } from './helpers/export-parity';
import { importFixture } from './helpers/media';

/* Phase 3b delivery with Use New Engine on (plan section 17): the export sheet
   offers every format at cutover and the five presets, writes a start
   timecode and reel, streams live previews, asks before exporting offline
   media (section 14.8), and the Render Queue panel runs jobs unattended from
   snapshots, keeps going past a failed job, and keeps user jobs across a
   relaunch. Run serially under the Electron lock. */

const ffprobe = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const probe = (file: string) => JSON.parse(execFileSync(ffprobe, [
  '-v', 'error', '-show_entries',
  'stream=codec_name,profile,pix_fmt,nb_frames,color_transfer,color_range:stream_tags=timecode,reel_name,alpha_mode:format_tags=encoder',
  '-of', 'json', file
], { encoding: 'utf8' }));

async function engineReady(app: ElectronApplication): Promise<void> {
  await requestJobEngine(app);
  await expect.poll(() => app.evaluate(() => {
    const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
    return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
  }), { timeout: 30_000 }).toBe('ready/ready');
}

/** The next save panels answer these paths, in order; returns how many were asked. */
async function stubSavePanels(app: ElectronApplication, files: string[]): Promise<void> {
  await app.evaluate(({ dialog }, paths) => {
    const g = globalThis as { __savePanels?: string[]; __saveAsked?: Array<Record<string, unknown>> };
    g.__savePanels = [...paths];
    g.__saveAsked = [];
    dialog.showSaveDialog = (async (...args: unknown[]) => {
      const options = (args.length > 1 ? args[1] : args[0]) as Record<string, unknown>;
      g.__saveAsked!.push(options);
      const filePath = g.__savePanels!.shift();
      return filePath ? { canceled: false, filePath } : { canceled: true, filePath: '' };
    }) as typeof dialog.showSaveDialog;
  }, files);
}

const asked = (app: ElectronApplication) => app.evaluate(() => (globalThis as { __saveAsked?: Array<Record<string, unknown>> }).__saveAsked ?? []);

async function solidProject(page: Page, name: string, extra: Array<Record<string, unknown>> = []): Promise<void> {
  await page.evaluate(({ name, extra }) => {
    const PM = (window as any).PM;
    PM.proj = PM.mkProject({ name, w: 320, h: 180, fps: 24, dur: 1 });
    PM.proj.work = [0, 1];
    PM.bus.emit('project');
    PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 200, h: 120 }, duration: 1 }, ...extra], { label: 'Layers', origin: 'user' });
  }, { name, extra });
}

const exportDone = (page: Page) => page.waitForFunction(() => !(window as any).PM.Export.busy, undefined, { timeout: 60_000 });

async function trackNativeOfflineExport(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as any;
    w.__offlineNativeStatuses = [];
    w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => w.__offlineNativeStatuses.push({ state: s.state, routed: !!s.routed }));
  });
}
async function expectNativeOfflineExport(page: Page): Promise<void> {
  const statuses = await page.evaluate(() => (window as any).__offlineNativeStatuses as Array<{ state: string; routed: boolean }>);
  expect(statuses.some(status => status.state === 'done')).toBe(true);
  expect(statuses.some(status => status.routed)).toBe(false);
}


test('the export sheet writes the new formats, presets, timecode and reel, with live previews', async () => {
  test.setTimeout(240_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-delivery-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  const { app, page } = session;
  try {
    await engineReady(app);
    await session.openEditor();
    await solidProject(page, 'Delivery');
    // Record the job engine's statuses as the window sees them.
    await page.evaluate(() => {
      const w = window as any;
      w.__statuses = [];
      w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => w.__statuses.push({ state: s.state, preview: !!s.preview, frame: s.preview?.frame ?? null }));
    });

    // Master · ProRes 422 HQ from the preset, with a start timecode and reel.
    await page.evaluate(() => (window as any).PM.Export.dialog());
    const dialog = page.getByRole('dialog', { name: 'Export', exact: true });
    await expect(dialog).toBeVisible();
    const formats = await dialog.locator('select[aria-label="Format"] option').allTextContents();
    expect(formats).toEqual(expect.arrayContaining(['QuickTime · ProRes 422 HQ', 'QuickTime · ProRes 4444 XQ', 'MP4 · H.264', 'HEVC', 'WebM · VP9', 'WebM · VP8']));
    const presets = await dialog.locator('select[aria-label="Saved preset"] option').allTextContents();
    expect(presets.slice(1, 6)).toEqual(['Master · ProRes 422 HQ', 'Master with alpha · ProRes 4444', 'Web · H.264', 'Web · HEVC', 'Image sequence · PNG']);
    await dialog.locator('select[aria-label="Saved preset"]').selectOption({ label: 'Master · ProRes 422 HQ' });
    await expect(dialog.locator('select[aria-label="Format"]')).toHaveValue('prores-hq');
    await expect(dialog.getByText('Quality', { exact: true })).toBeHidden();
    await expect(dialog.getByText('Start timecode', { exact: true })).toBeVisible();
    await dialog.getByLabel('Start timecode').fill('01:00:00:00');
    await dialog.getByLabel('Start timecode').press('Enter');
    await dialog.getByLabel('Reel name').fill('A001');
    await dialog.getByLabel('Reel name').press('Enter');
    await expect(dialog.locator('.export-summary')).toContainText('≈');
    await expect(dialog.locator('.export-summary')).toContainText('ProRes 422 HQ');
    const master = path.join(root, 'master.mov');
    await stubSavePanels(app, [master]);
    await dialog.getByRole('button', { name: 'Export video', exact: true }).click();
    await exportDone(page);
    const hq = probe(master);
    const video = hq.streams.find((s: any) => s.codec_name === 'prores');
    expect(video).toMatchObject({ profile: 'HQ', nb_frames: '24', color_transfer: 'iec61966-2-1', color_range: 'tv' });
    const tmcd = hq.streams.find((s: any) => s.tags?.timecode);
    expect(tmcd?.tags).toMatchObject({ timecode: '01:00:00:00', reel_name: 'A001' });
    expect((await asked(app)).length).toBe(1);
    const statuses = await page.evaluate(() => (window as any).__statuses);
    expect(statuses.at(-1).state).toBe('done');
    // The progress sheet's live preview: encoded frames arrive with the statuses.
    expect(statuses.some((s: any) => s.preview)).toBe(true);
    expect(statuses.filter((s: any) => s.preview).at(-1).frame).toBe(23);

    // HEVC 10-bit in QuickTime through the sheet's rows.
    await page.evaluate(() => (window as any).PM.Export.dialog());
    await dialog.locator('select[aria-label="Format"]').selectOption('hevc');
    await dialog.locator('select[aria-label="Container"]').selectOption('mov');
    await dialog.locator('select[aria-label="Bit depth"]').selectOption('high');
    await expect(dialog.getByText('Transparent background')).toBeHidden();
    const hevc = path.join(root, 'hevc10.mov');
    await stubSavePanels(app, [hevc]);
    await dialog.getByRole('button', { name: 'Export video', exact: true }).click();
    await exportDone(page);
    expect(probe(hevc).streams.find((s: any) => s.codec_name === 'hevc')).toMatchObject({ pix_fmt: 'yuv420p10le', nb_frames: '24' });

    // A 16-bit PNG sequence with transparency: the save panel names a folder.
    await page.evaluate(() => (window as any).PM.Export.dialog());
    await dialog.getByRole('button', { name: 'Images', exact: true }).click();
    await dialog.locator('select[aria-label="Format"]').selectOption('png');
    await dialog.locator('select[aria-label="Bit depth"]').selectOption('high');
    const alpha = dialog.getByRole('switch', { name: 'Transparent background', exact: true });
    if (await alpha.getAttribute('aria-checked') !== 'true') await alpha.click();
    const frames = path.join(root, 'Frames');
    await stubSavePanels(app, [frames]);
    await dialog.getByRole('button', { name: 'Export frames', exact: true }).click();
    await exportDone(page);
    const files = (await readdir(frames)).sort();
    expect(files).toHaveLength(24);
    expect(files[0]).toBe('Frames_00000.png');
    expect(probe(path.join(frames, files[5]!)).streams[0]).toMatchObject({ codec_name: 'png', pix_fmt: 'rgba64be' });

    // WebM · VP8, frame-exact.
    const vp8 = path.join(root, 'web.webm');
    await stubSavePanels(app, [vp8]);
    expect(await page.evaluate(() => (window as any).PM.Export.run({ format: 'webm-vp8', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false }))).toEqual({ cancelled: false });
    expect(probe(vp8).streams[0]).toMatchObject({ codec_name: 'vp8' });
    // Nothing temporary remains beside the outputs.
    expect((await readdir(root)).sort()).toEqual(['Frames', 'hevc10.mov', 'master.mov', 'web.webm']);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});

test('an export with offline media asks first with the native message', async () => {
  test.setTimeout(120_000);
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  const { app, page } = session;
  try {
    await engineReady(app);
    await session.openEditor();
    await solidProject(page, 'Offline');
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj.assets = { ...(PM.proj.assets || {}), gone: { id: 'gone', name: 'A001_C003.mov', kind: 'video', format: 'mov', w: 320, h: 180, dur: 1, fps: 24 } };
      PM.proj.layers.push({ ...PM.mkLayer('video', { d: { asset: 'gone', w: 320, h: 180 } }), id: 'offline-clip', name: 'A001_C003.mov', from: 0, dur: 1 });
      PM.touch();
      (window as any).__located = [];
      PM.pickFiles = (_sequence: boolean, options: { replaceAssetId?: string }) => (window as any).__located.push(options?.replaceAssetId);
    });
    await app.evaluate(({ dialog }) => {
      const g = globalThis as { __messages?: Array<Record<string, unknown>>; __answer?: number };
      g.__messages = [];
      dialog.showMessageBox = (async (...args: unknown[]) => {
        const options = (args.length > 1 ? args[1] : args[0]) as Record<string, unknown>;
        g.__messages!.push(options);
        return { response: g.__answer ?? 1, checkboxChecked: false };
      }) as typeof dialog.showMessageBox;
    });
    await stubSavePanels(app, []);
    const options = { format: 'prores-hq', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false };
    // Cancel: nothing is asked or written.
    await app.evaluate(() => { (globalThis as { __answer?: number }).__answer = 2; });
    expect(await page.evaluate((o) => (window as any).PM.Export.run(o), options)).toEqual({ cancelled: true });
    const messages = await app.evaluate(() => (globalThis as { __messages?: Array<Record<string, unknown>> }).__messages);
    expect(messages).toHaveLength(1);
    expect(messages![0]).toMatchObject({ message: '1 clip is offline', buttons: ['Locate Media…', 'Export Without Missing Media', 'Cancel'] });
    expect(String(messages![0]!['detail'])).toContain('A001_C003.mov');
    expect(await asked(app)).toEqual([]);
    // Locate Media…: the Media panel's locate flow for the clip.
    await app.evaluate(() => { (globalThis as { __answer?: number }).__answer = 0; });
    expect(await page.evaluate((o) => (window as any).PM.Export.run(o), options)).toEqual({ cancelled: true });
    expect(await page.evaluate(() => (window as any).__located)).toEqual(['gone']);
    expect(await asked(app)).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

for (const setting of ['0', '1'] as const) {
  test(`offline export choices preserve the missing-media picture with new engine ${setting}`, async () => {
    test.setTimeout(180_000);
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-offline-export-'));
    let session: Awaited<ReturnType<typeof launchApp>> | undefined;
    try {
      session = await launchApp({ userData: path.join(root, 'profile'), env: { POWERMOVE_NEW_ENGINE: setting } });
      if (setting === '1') await engineReady(session.app);
      await session.openEditor();
      await solidProject(session.page, 'Offline choices');
      if (setting === '1') await trackNativeOfflineExport(session.page);
      await session.page.evaluate(() => {
        const PM = (window as any).PM;
        PM.proj.assets = { ...PM.proj.assets, gone: { id: 'gone', name: 'Missing.mov', kind: 'video', format: 'mov', w: 320, h: 180, dur: 1, fps: 24 } };
        PM.proj.layers.push({ ...PM.mkLayer('video', { d: { asset: 'gone', w: 320, h: 180 } }), id: 'offline-clip', from: 0, dur: 1 });
        PM.touch();
        (window as any).__located = [];
        PM.pickFiles = (_sequence: boolean, options: { replaceAssetId?: string }) => (window as any).__located.push(options?.replaceAssetId);
      });
      await session.app.evaluate(({ dialog }) => {
        const g = globalThis as { __answer?: number; __messages?: Array<Record<string, unknown>> };
        g.__messages = [];
        dialog.showMessageBox = (async (...args: unknown[]) => {
          g.__messages!.push((args.length > 1 ? args[1] : args[0]) as Record<string, unknown>);
          return { response: g.__answer ?? 2, checkboxChecked: false };
        }) as typeof dialog.showMessageBox;
      });
      await stubSavePanels(session.app, []);
      const options = { format: 'prores', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false };
      await session.app.evaluate(() => { (globalThis as { __answer?: number }).__answer = 2; });
      expect(await session.page.evaluate((o) => (window as any).PM.Export.run(o), options)).toEqual({ cancelled: true });
      expect(await asked(session.app)).toEqual([]);
      await session.app.evaluate(() => { (globalThis as { __answer?: number }).__answer = 0; });
      expect(await session.page.evaluate((o) => (window as any).PM.Export.run(o), options)).toEqual({ cancelled: true });
      expect(await session.page.evaluate(() => (window as any).__located)).toEqual(['gone']);
      const missing = path.join(root, 'missing.mov');
      await stubSavePanels(session.app, [missing]);
      await session.app.evaluate(() => { (globalThis as { __answer?: number }).__answer = 1; });
      expect(await session.page.evaluate((o) => (window as any).PM.Export.run(o), options)).toEqual({ cancelled: false });
      expect((await stat(missing)).size).toBeGreaterThan(1000);
      if (setting === '1') await expectNativeOfflineExport(session.page);
      const prompts = await session.app.evaluate(() => (globalThis as { __messages?: Array<Record<string, unknown>> }).__messages ?? []);
      expect(prompts).toHaveLength(3);
      expect(prompts[0]).toMatchObject({ buttons: ['Locate Media…', 'Export Without Missing Media', 'Cancel'] });
      await session.page.evaluate(() => {
        const PM = (window as any).PM;
        PM.proj.layers = PM.proj.layers.filter((layer: { id: string }) => layer.id !== 'offline-clip');
        PM.touch();
      });
      const omitted = path.join(root, 'omitted.mov');
      await stubSavePanels(session.app, [omitted]);
      expect(await session.page.evaluate((o) => (window as any).PM.Export.run(o), options)).toEqual({ cancelled: false });
      const decoded = (file: string) => execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-f', 'rawvideo', '-pix_fmt', 'rgba', 'pipe:1'], { maxBuffer: 16 * 1024 * 1024 });
      expect(decoded(missing).equals(decoded(omitted))).toBe(true);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      try { await session?.close(); } finally { await rm(root, { recursive: true, force: true }); }
    }
  });
}

test('offline exports preserve decoded media and delivery properties with the new engine on', async ({}, testInfo) => {
  test.setTimeout(420_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-offline-parity-'));
  const exports: Record<string, Record<string, string>> = { prores: {}, mp4: {}, webm: {} };
  const webmImports: Record<string, { width: number; height: number; duration: number }> = {};
  try {
    for (const setting of ['0', '1']) {
      let session: Awaited<ReturnType<typeof launchApp>> | undefined;
      try {
        session = await launchApp({ userData: path.join(root, `profile-${setting}`), env: { POWERMOVE_NEW_ENGINE: setting } });
        if (setting === '1') await engineReady(session.app);
        await session.openEditor();
        await solidProject(session.page, 'Offline parity');
        await importFixture(session.page, 'tone.wav');
        await session.page.waitForFunction(() => (window as any).PM.proj.layers.some(
          (layer: any) => layer.type === 'audio' && layer.name === 'tone.wav',
        ));
        if (setting === '1') await trackNativeOfflineExport(session.page);
        await session.page.evaluate(() => {
          const PM = (window as any).PM;
          PM.proj.assets.gone = { id: 'gone', name: 'Missing.mov', kind: 'video', format: 'mov', w: 320, h: 180, dur: 1, fps: 24 };
          PM.proj.layers.push({ ...PM.mkLayer('video', { d: { asset: 'gone', w: 320, h: 180 } }), id: 'offline-clip', from: 0, dur: 1 });
          PM.touch();
        });
        await session.app.evaluate(({ dialog }) => {
          dialog.showMessageBox = (async () => ({ response: 1, checkboxChecked: false })) as typeof dialog.showMessageBox;
        });
        for (const format of ['prores', 'mp4', 'webm'] as const) {
          const file = path.join(root, `export-${format}-${setting}.${format === 'prores' ? 'mov' : format}`);
          await stubSavePanels(session.app, [file]);
          if (setting === '1') await session.page.evaluate(() => { (window as any).__offlineNativeStatuses = []; });
          expect(await session.page.evaluate((selected) => (window as any).PM.Export.run({
            format: selected, scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: true,
          }), format)).toEqual({ cancelled: false });
          if (setting === '1') await expectNativeOfflineExport(session.page);
          exports[format]![setting] = file;
          if (format === 'webm') {
            const bytes = [...await readFile(file)];
            webmImports[setting] = await session.page.evaluate(async (data) => {
              const video = document.createElement('video');
              if (!video.canPlayType('video/webm; codecs="vp9"')) throw new Error('Chromium cannot import VP9 WebM');
              const url = URL.createObjectURL(new Blob([Uint8Array.from(data)], { type: 'video/webm' }));
              try {
                video.preload = 'auto'; video.muted = true; video.src = url;
                await new Promise<void>((resolve, reject) => {
                  video.addEventListener('loadeddata', () => resolve(), { once: true });
                  video.addEventListener('error', () => reject(new Error('Chromium could not decode WebM')), { once: true });
                });
                const initial = { width: video.videoWidth, height: video.videoHeight, duration: video.duration };
                await new Promise<void>((resolve, reject) => {
                  video.addEventListener('seeked', () => resolve(), { once: true });
                  video.addEventListener('error', () => reject(new Error('Chromium could not seek WebM')), { once: true });
                  video.currentTime = Math.max(0, video.duration - 1 / 24);
                });
                return initial;
              } finally { video.removeAttribute('src'); video.load(); URL.revokeObjectURL(url); }
            }, bytes);
          }
        }
      } finally { await session?.close(); }
    }
    for (const format of ['prores', 'mp4', 'webm'] as const) for (const setting of ['0', '1']) {
      await testInfo.attach(`offline-${format}-${setting}`, { path: exports[format]![setting]!, contentType: format === 'webm' ? 'video/webm' : format === 'mp4' ? 'video/mp4' : 'video/quicktime' });
    }
    // G-X1: the engine keeps today's stream properties (pictures against the
    // reference are export-encoding-parity.spec.ts's).
    for (const format of ['prores', 'mp4', 'webm'] as const) {
      const { checks } = compareExports(exports[format]!['0']!, exports[format]!['1']!, null, 24, true);
      expect(checks.filter(check => !check.equal), `${format} G-X1`).toEqual([]);
    }
    for (const format of ['prores', 'mp4'] as const) {
      const legacyImport = avFoundationImport(exports[format]!['0']!);
      const nativeImport = avFoundationImport(exports[format]!['1']!);
      for (const imported of [legacyImport, nativeImport]) {
        expect(imported).toMatchObject({ readable: true, playable: true, frames: 24, firstPTS: 0, duration: 1, videoDuration: 1 });
        expect(imported.audioSamples, 'AVFoundation decoded audio samples').toBeGreaterThan(0);
      }
      expect(nativeImport.audioSamples, 'AVFoundation audio sample count').toBe(legacyImport.audioSamples);
      if (format === 'prores') {
        expect(nativeImport.timecodeTrackStart, 'AVFoundation timecode start').toBe(0);
        expect(nativeImport.timecodeTrackDuration, 'AVFoundation timecode duration').toBe(legacyImport.videoDuration);
      }
    }
    expect(probe(exports.prores!['1']!).streams.find((stream: any) => stream.tags?.timecode)?.tags?.timecode).toBe('00:00:00:00');
    // RDD 36 byte 21: aspect ratio unspecified in both; today's FFmpeg leaves
    // the frame-rate code unspecified, VideoToolbox writes 2 (24 fps).
    expect(proResFrameHeaderCodes(exports.prores!['0']!)).toEqual({ aspectRatio: 0, frameRate: 0 });
    expect(proResFrameHeaderCodes(exports.prores!['1']!)).toEqual({ aspectRatio: 0, frameRate: 2 });
    // Chromium reads the engine's WebM at the project's size and length: its
    // duration is the longest track's end (today's adds a frame, 25/24 s).
    expect(webmImports['1']).toMatchObject({ width: 320, height: 180 });
    expect(Math.abs(webmImports['1']!.duration - 1), 'WebM duration').toBeLessThan(1 / 24);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('the render queue runs jobs unattended, keeps going past a failure, and keeps user jobs across a relaunch', async () => {
  test.setTimeout(240_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-queue-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await engineReady(session.app);
    await session.openEditor();
    await solidProject(session.page, 'Queue A');
    const a = path.join(root, 'A.mov');
    const locked = path.join(root, 'Locked');
    await mkdir(locked);
    const failing = path.join(locked, 'Failing.mp4');
    const b = path.join(root, 'B.webm');
    await stubSavePanels(session.app, [a, failing, b]);
    const add = (o: Record<string, unknown>) => session.page.evaluate((o) => (window as any).PM.Export.addToQueue(o), o);
    expect(await add({ format: 'prores-422', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false, preset: 'Mine' })).toMatchObject({ status: 'added' });
    // A job whose destination folder is read-only when it runs fails; the
    // queue goes on.
    await solidProject(session.page, 'Queue failing');
    expect(await add({ format: 'h264', scale: 1, fps: 24, range: 'all', quality: 'draft', mblur: false, audio: false })).toMatchObject({ status: 'added' });
    // Snapshots: editing the live project after queuing does not change the job.
    await solidProject(session.page, 'Queue B');
    expect(await add({ format: 'webm-vp9', scale: 0.5, fps: 24, range: 'all', quality: 'draft', mblur: false, audio: false })).toMatchObject({ status: 'added' });
    await session.page.evaluate(() => { const PM = (window as any).PM; PM.proj.name = 'Edited after queuing'; PM.touch(); });

    await chmod(locked, 0o555);
    await session.page.evaluate(() => (window as any).PM.Export.queueDialog());
    const panel = session.page.getByRole('dialog', { name: 'Render Queue', exact: true });
    await expect(panel).toBeVisible();
    const rows = panel.locator('.render-queue-job');
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText('A.mov');
    await expect(rows.nth(0)).toContainText('Mine · QuickTime · ProRes 422');
    await expect(rows.nth(0)).toContainText('Waiting to start');
    await panel.getByRole('button', { name: 'Start Queue', exact: true }).click();
    await expect(rows.nth(2)).toContainText('Done', { timeout: 120_000 });
    await expect(rows.nth(0)).toContainText('Done');
    await expect(rows.nth(1)).toContainText('Failed');
    await chmod(locked, 0o755);
    expect(await readdir(locked)).toEqual([]);
    await rm(locked, { recursive: true });
    expect(probe(a).streams.find((s: any) => s.codec_name === 'prores')).toMatchObject({ profile: 'Standard', nb_frames: '24' });
    expect(probe(b).streams[0]).toMatchObject({ codec_name: 'vp9' });
    await expect(panel.getByRole('button', { name: 'Start Queue', exact: true })).toBeDisabled();
    // A pending job and the finished ones survive a relaunch.
    const c = path.join(root, 'C.mov');
    await stubSavePanels(session.app, [c]);
    await panel.getByRole('button', { name: 'Close', exact: true }).click();
    expect(await add({ format: 'prores-proxy', scale: 0.5, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false })).toMatchObject({ status: 'added' });
    await session.relaunch();
    await engineReady(session.app);
    await session.page.evaluate(() => (window as any).PM.Export.queueDialog());
    const again = session.page.getByRole('dialog', { name: 'Render Queue', exact: true });
    const kept = again.locator('.render-queue-job');
    await expect(kept).toHaveCount(4);
    await expect(kept.nth(3)).toContainText('Waiting to start');
    await expect(kept.nth(0)).toContainText('Done');
    await again.getByRole('button', { name: 'Start Queue', exact: true }).click();
    await expect(kept.nth(3)).toContainText('Done', { timeout: 120_000 });
    expect(probe(c).streams.find((s: any) => s.codec_name === 'prores')).toMatchObject({ profile: 'Proxy' });
    // The job menu: remove a finished job.
    await kept.nth(1).getByRole('button', { name: /actions$/ }).click();
    await session.page.getByRole('menuitem', { name: 'Remove' }).click();
    await expect(kept).toHaveCount(3);
    expect((await readdir(root)).sort()).toEqual(['A.mov', 'B.webm', 'C.mov']);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
