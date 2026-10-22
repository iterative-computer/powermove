/*
 * The viewer's engine presentation through GPU trouble, behind "Use New
 * Engine" (docs/engine/bridge.md, "Failure handling"; phase2-gate.md D3).
 *
 * A WebGL context loss in the page (the GPU process restarting) also loses
 * the presentation's WebGPU device: drawing with it again crashed the
 * renderer (Electron 44, the presentation worker), so the presentation
 * replaces its drawing. And while this page's main thread is held up for
 * more than a second (a GPU restart, a long task), a surface transfer
 * times out on main's side yet still lands here, main sends it again, and
 * a second release notice reached main after it had dropped the texture
 * (phase2-gate.md D3: "Shared texture with id … not found"). Neither may
 * surface as a page error, and the viewer must present engine frames
 * again afterwards.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-present-recovery.spec.ts --workers=1
 */
import { writeFile } from 'node:fs/promises';
import type { Page } from 'playwright';

import { DEVICE_LOSS_WINDOW_MS, MAX_DEVICE_LOSSES } from '../src/extensions/viewer/engine-present/presentation';
import { expect, test, type LaunchedApp } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };

/** A 640×360, 30 fps, 4 s composition of solids (the engine draws it). */
async function solidsProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Recovery', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 80, h: 40 } }, project);
    box.p['position.x'].v = 200; box.p['position.y'].v = 120;
    project.layers = [box];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

const presenting = (page: Page): Promise<boolean> => page.evaluate(() => {
  const canvas = document.getElementById('engine-present');
  return !!canvas && canvas.style.visibility === 'visible';
});

/** The newest engine frame's sequence number on screen. */
const latestSeq = (page: Page): Promise<number> => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.engineFrame?.()?.seq ?? -1);

async function openEngineViewer(session: LaunchedApp): Promise<void> {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
  await solidsProject(page);
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await expect.poll(() => presenting(page), { timeout: 20_000 }).toBe(true);
}

/** The window's pixel at a point of the stage (capturePage of the hidden window, never the screen). */
async function stagePixel(session: LaunchedApp, cssX: number, cssY: number): Promise<number[]> {
  const origin = await session.page.evaluate(() => {
    const rect = (window as any).PM.Kernel.services.get('viewer').stage.getBoundingClientRect();
    return { x: rect.left, y: rect.top };
  });
  return session.app.evaluate(async ({ BrowserWindow }, at) => {
    const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
    const bitmap = (await window.webContents.capturePage({ x: Math.round(at.x), y: Math.round(at.y), width: 1, height: 1 })).toBitmap();
    return [bitmap[2]!, bitmap[1]!, bitmap[0]!];
  }, { x: origin.x + cssX, y: origin.y + cssY });
}

/** The center of the box layer on screen, from the frame's overlay geometry (stage CSS px). */
async function boxCenter(page: Page): Promise<{ x: number; y: number }> {
  return page.evaluate(() => {
    const frame = (window as any).PM.Kernel.services.get('viewer').engineFrame();
    const quad: [number, number][] = frame.overlay.layers[0].bounds;
    return { x: quad.reduce((a, p) => a + p[0], 0) / 4, y: quad.reduce((a, p) => a + p[1], 0) / 4 };
  });
}

/** Kills the GPU process (Electron starts a new one): every context in the page is lost. */
async function killGpuProcess(session: LaunchedApp): Promise<number> {
  return session.app.evaluate(({ app }) => {
    const gpu = app.getAppMetrics().find((m) => m.type === 'GPU');
    if (!gpu) throw new Error('no GPU process');
    process.kill(gpu.pid, 'SIGKILL');
    return gpu.pid;
  });
}

/** A new viewer size (a new surface pool): the old pool's imports are released. */
async function resizeViewer(session: LaunchedApp, width: number): Promise<void> {
  await session.app.evaluate(({ BrowserWindow }, width) => {
    const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'));
    const bounds = window!.getBounds();
    window!.setBounds({ ...bounds, width });
  }, width);
}

test.describe('Use New Engine: engine presentation recovery', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('a GPU process crash while the engine presents ends in engine frames again, without page errors', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    const pid = await killGpuProcess(session);
    // The page's WebGL context is lost and restored; the presentation makes
    // a new F16 painter for its canvas.
    await expect.poll(() => session.app.evaluate(({ app }) => app.getAppMetrics().find((m) => m.type === 'GPU')?.pid ?? 0), { timeout: 20_000 }).not.toBe(pid);
    // Pool replacements after the crash release the old imports.
    for (const width of [1180, 1240, 1200]) {
      await resizeViewer(session, width);
      await page.waitForTimeout(400);
    }
    // An edit makes a new engine frame, which must reach the screen.
    const before = await latestSeq(page);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers[0];
      PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 50 }, { label: 'Opacity', origin: 'inspector' });
    });
    try {
      await expect.poll(() => latestSeq(page), { timeout: 20_000 }).toBeGreaterThan(before);
    } catch (error) {
      console.log('GPU RECOVERY DIAGNOSTIC', JSON.stringify(session.diagnostics.console.filter((r) => /presentation|painter|graphics|WebGL/.test(r.text)).map((r) => r.text.slice(0, 300))));
      throw error;
    }
    await expect.poll(() => presenting(page), { timeout: 20_000 }).toBe(true);
    // The edited frame is what the window shows: #FF3300 at 50% over #203040.
    const center = await boxCenter(page);
    await expect.poll(async () => {
      const [r, g, b] = await stagePixel(session, center.x, center.y);
      return Math.max(Math.abs(r! - 0x90), Math.abs(g! - 0x32), Math.abs(b! - 0x20));
    }, { timeout: 10_000 }).toBeLessThanOrEqual(4);
    // Observe retirement completion rather than sleeping through each run.
    await expect.poll(async () => {
      const status = await bridgeStatus(session);
      return status.allReleased === status.retired;
    }, { timeout: 10_000 }).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('surface transfers that time out on main and land late are released once, without page errors (D3)', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    const initialSlots = await page.evaluate(() => (window as any).__powermoveTestBridge.frameBridge.stats().held);
    // The next pool's first transfer meets a renderer whose main thread is
    // busy for 1.8 s: main's 1000 ms transfer timeout fires for the
    // transfers queued behind it, the bridge sends them again, and the
    // first copies land too.
    await session.app.evaluate(({ sharedTexture, BrowserWindow }) => {
      const module = sharedTexture as unknown as { sendSharedTexture: (...args: unknown[]) => Promise<void> };
      const original = module.sendSharedTexture.bind(module);
      let armed = true;
      module.sendSharedTexture = (...args: unknown[]) => {
        if (armed) {
          armed = false;
          const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
          void window.webContents.executeJavaScript('(() => { const t = performance.now(); while (performance.now() - t < 1800) {} return true; })()');
        }
        return original(...args);
      };
    });
    await resizeViewer(session, 1180);
    await expect.poll(() => bridgeStatus(session).then((s) => s.sendFailures), { timeout: 20_000 }).toBeGreaterThan(0);
    // Another pool retires the one delivered twice.
    await page.waitForTimeout(1500);
    await resizeViewer(session, 1260);
    await expect.poll(() => bridgeStatus(session).then((s) => s.retired), { timeout: 20_000 }).toBeGreaterThanOrEqual(initialSlots * 2);
    await page.waitForTimeout(2000);
    const bridge = await bridgeStatus(session);
    const receiver = await page.evaluate(() => (window as any).__powermoveTestBridge.frameBridge.stats());
    // Every copy main sent again arrived and was let go without a notice;
    // every retired surface was released by every process, exactly once.
    expect(receiver.duplicates).toBe(bridge.sendFailures);
    expect(receiver.lateReleases).toBe(0);
    expect(receiver.errors).toEqual([]);
    expect(bridge.allReleased).toBe(bridge.retired);
    expect(session.diagnostics.pageErrors).toEqual([]);
    // The viewer still presents the engine's frames of the new pool.
    await expect.poll(() => presenting(page), { timeout: 20_000 }).toBe(true);
  });
});

/*
 * Repeated GPU process crashes while frames keep arriving. Electron 44's
 * WebGPU importExternalTexture copied RGBA VideoFrames through Blink's
 * WebGpuSharedImageWrapperCache; once the shared raster context was lost
 * and recreated, the next import dereferenced a cached wrapper's null
 * shared image (SIGSEGV at 0x1c in the presentation worker). Every reset
 * here lands during playback, so imports race the loss.
 * PM_GPU_RESET_ITERATIONS sets the count (stress runs use 200). Chromium's
 * fallback to software compositing after three GPU crashes, and its
 * blocking of WebGL/WebGPU for the app's origin after repeated resets
 * (which also stops today's engine canvas), are turned off so every
 * iteration exercises the hardware path. Resets are paced so no
 * more than MAX_DEVICE_LOSSES land in DEVICE_LOSS_WINDOW_MS: beyond that
 * the presentation deliberately stops (a GPU that keeps failing must not
 * spin the viewer).
 */
const RESETS = Math.max(1, Number(process.env.PM_GPU_RESET_ITERATIONS || 3));

test.describe('Use New Engine: repeated GPU resets during playback', () => {
  test.use({ desktopLaunchOptions: { env: ON, chromiumSwitches: ['--disable-gpu-process-crash-limit', '--disable-domain-blocking-for-3d-apis'] } });

  test('never crash the renderer and end in engine frames each time', async ({ session }) => {
    test.setTimeout(120_000 + RESETS * 60_000);
    const resets: number[] = [];
    const { page } = session;
    await session.app.evaluate(({ app }) => {
      const gone: unknown[] = [];
      (globalThis as any).__pmRendererGone = gone;
      app.on('render-process-gone', (_event, _contents, details) => gone.push({ kind: 'renderer', ...details }));
      app.on('child-process-gone', (_event, details) => { if (details.type !== 'GPU') gone.push({ kind: 'child', ...details }); });
    });
    const gone = (): Promise<unknown[]> => session.app.evaluate(() => (globalThis as any).__pmRendererGone);
    await openEngineViewer(session);
    const play = () => page.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.playing) { PM.setTime(0); PM.play(); }
    });
    for (let iteration = 0; iteration < RESETS; iteration++) {
      await play();
      const playing = await latestSeq(page);
      await expect.poll(() => latestSeq(page), { timeout: 20_000 }).toBeGreaterThan(playing);
      const window = resets.at(-MAX_DEVICE_LOSSES);
      if (window !== undefined) await page.waitForTimeout(Math.max(0, window + DEVICE_LOSS_WINDOW_MS + 1_000 - Date.now()));
      resets.push(Date.now());
      const pid = await killGpuProcess(session);
      await expect.poll(() => session.app.evaluate(({ app }) => app.getAppMetrics().find((m) => m.type === 'GPU')?.pid ?? 0), { timeout: 20_000 }).not.toBe(pid);
      // A new pool after the loss, as a window change makes one.
      await resizeViewer(session, iteration % 2 ? 1180 : 1240);
      await play();
      const after = await latestSeq(page);
      try {
        await expect.poll(() => latestSeq(page), { timeout: 30_000 }).toBeGreaterThan(after);
      } catch (error) {
        console.log('GPU RESET DIAGNOSTIC', iteration + 1, JSON.stringify(await page.evaluate(() => {
          const PM = (window as any).PM, presentation = PM.Kernel.services.get('viewer')?.enginePresentation;
          return { playing: PM.playing, time: PM.time, connection: PM.EngineConnection?.status?.(), presenting: presentation?.presenting,
            stats: presentation?.stats?.(), state: presentation?.state, receiver: (window as any).__powermoveTestBridge?.frameBridge?.stats?.() };
        }).catch((e: unknown) => ({ error: String(e) }))), JSON.stringify(await bridgeStatus(session).catch((e: unknown) => String(e))));
        throw error;
      }
      await expect.poll(() => presenting(page), { timeout: 30_000 }).toBe(true);
      expect(await gone(), `iteration ${iteration + 1}`).toEqual([]);
      expect(session.diagnostics.pageErrors, `iteration ${iteration + 1}`).toEqual([]);
      if ((iteration + 1) % 10 === 0 || iteration + 1 === RESETS) console.log(`GPU reset ${iteration + 1}/${RESETS} recovered`);
    }
    await page.evaluate(() => (window as any).PM.pause());
    await expect.poll(() => page.evaluate(() => (window as any).PM.playing), { timeout: 10_000 }).toBe(false);
    // An edit's frame is what the window shows: #FF3300 at 50% over #203040.
    const before = await latestSeq(page);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'opacity', value: 50 }, { label: 'Opacity', origin: 'inspector' });
    });
    await expect.poll(() => latestSeq(page), { timeout: 20_000 }).toBeGreaterThan(before);
    await expect.poll(() => presenting(page), { timeout: 20_000 }).toBe(true);
    const center = await boxCenter(page);
    try {
      await expect.poll(async () => {
        const [r, g, b] = await stagePixel(session, center.x, center.y);
        return Math.max(Math.abs(r! - 0x90), Math.abs(g! - 0x32), Math.abs(b! - 0x20));
      }, { timeout: 10_000 }).toBeLessThanOrEqual(4);
    } catch (error) {
      const png = await session.app.evaluate(async ({ BrowserWindow }) => {
        const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
        return (await window.webContents.capturePage()).toPNG().toString('base64');
      });
      await writeFile(test.info().outputPath('window.png'), Buffer.from(png, 'base64'));
      console.log('GPU RESET PIXEL', JSON.stringify(center), JSON.stringify(await stagePixel(session, center.x, center.y)));
      throw error;
    }
    await expect.poll(async () => {
      const status = await bridgeStatus(session);
      return status.allReleased === status.retired;
    }, { timeout: 15_000 }).toBe(true);
    expect(await gone()).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

type BridgeStatus = { sendFailures: number; retired: number; allReleased: number };

const bridgeStatus = (session: LaunchedApp): Promise<BridgeStatus> =>
  session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.frameBridge.launch.status());
