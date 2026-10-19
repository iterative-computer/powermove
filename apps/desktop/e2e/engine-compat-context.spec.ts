/*
 * `render.gl.context`, the compatibility WebGL2 context, with "Use New
 * Engine" on (owner amendment 23; plan section 20; kernel/compat-context.ts;
 * docs/engine/extensions-compat.md "The compatibility context").
 *
 * Today an extension that asks for `render.gl.context` gets the compositor's
 * context: `readPixels` on its default framebuffer reads the frame the
 * viewer shows, and drawing into it shows in the viewer until the next
 * frame. With the engine, the context is on an offscreen canvas: the
 * presented engine frame is uploaded into it before it is read or drawn,
 * and what the extension draws is composited over the engine's canvas until
 * the next frame. Compared with today's compositor in the same window (its
 * context on the same bound canvas, scaffolding until routing is removed).
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-compat-context.spec.ts --workers=1
 */
import { writeFile } from 'node:fs/promises';
import type { Page } from 'playwright';

import { expect, test } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };

async function project(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Compatibility context', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 160, h: 90 } }, project);
    box.p['position.x'].v = 220; box.p['position.y'].v = 140;
    const disc = PM.mkLayer('shape', { name: 'Disc', from: 0, dur: 4, d: { shape: 'ellipse', w: 120, h: 120, color: '#33CC66', stroke: 0 } }, project);
    disc.p['position.x'].v = 440; disc.p['position.y'].v = 220;
    project.layers = [box, disc];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

test.describe('Use New Engine: the compatibility context', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('reads back the frame on screen and shows what an extension draws until the next frame', async ({ session }, testInfo) => {
    test.setTimeout(120_000);
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
    await project(page);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
    // Let the view settle (fit, fonts): the frames after this are the test's own.
    await page.waitForTimeout(800);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('compat-context-probe');
      /** Moves the playhead one frame and waits for the engine to present it. */
      const nextFrame = (log: string) => new Promise<boolean>((resolve) => {
        const off: any = api.events.on('frame:rendered', () => { if (typeof off === 'function') off(); else off?.dispose?.(); clearTimeout(timer); resolve(true); });
        const timer = setTimeout(() => { if (typeof off === 'function') off(); else off?.dispose?.(); (out.timeouts as string[]).push(log); resolve(false); }, 5000);
        PM.setTime(Math.round(PM.time * 30 + 1) / 30);
      });
      const gl: WebGL2RenderingContext = api.render.gl.context;
      const out: Record<string, unknown> = { timeouts: [] };
      out.identity = {
        isContext: gl instanceof WebGL2RenderingContext,
        sameObject: api.render.gl.context === gl,
        canvasIsBound: gl.canvas === api.render.gl.canvas,
        attributes: gl.getContextAttributes(),
        offscreen: (gl as any).compatCanvas instanceof OffscreenCanvas
      };
      const W = gl.drawingBufferWidth, H = gl.drawingBufferHeight;
      out.size = { W, H, boundW: (api.render.gl.canvas as HTMLCanvasElement).width, boundH: (api.render.gl.canvas as HTMLCanvasElement).height };
      // A fresh engine frame, then read the default framebuffer.
      await nextFrame('readback');
      const engine = new Uint8Array(W * H * 4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.readPixels(0, 0, W, H, gl.RGBA, gl.UNSIGNED_BYTE, engine);
      // The frame on screen, copied from the engine's canvas the plain way
      // (top-down), to check the upload's orientation, size and color path.
      const presentNow = document.getElementById('engine-present') as HTMLCanvasElement;
      const s = presentNow.getBoundingClientRect(), b = (api.render.gl.canvas as HTMLCanvasElement).getBoundingClientRect();
      const k = presentNow.width / s.width;
      out.geometry = { dpr: devicePixelRatio, source: [presentNow.width, presentNow.height], sourceBox: [s.left, s.top, s.width, s.height], boundBox: [b.left, b.top, b.width, b.height], from: [(b.left - s.left) * k, (b.top - s.top) * k] };
      const onScreen = document.createElement('canvas'); onScreen.width = W; onScreen.height = H;
      const oc = onScreen.getContext('2d')!;
      oc.fillStyle = '#000'; oc.fillRect(0, 0, W, H); oc.imageSmoothingEnabled = false;
      oc.drawImage(presentNow, Math.round((b.left - s.left) * k), Math.round((b.top - s.top) * k), W, H, 0, 0, W, H);
      const onScreenPixels = oc.getImageData(0, 0, W, H).data;
      const flipped = new Uint8Array(W * H * 4);
      for (let y = 0; y < H; y++) flipped.set(engine.subarray((H - 1 - y) * W * 4, (H - y) * W * 4), y * W * 4);
      // Draw into it: a red square in the lower left (GL's origin), as an extension would on frame:rendered.
      gl.enable(gl.SCISSOR_TEST);
      gl.scissor(16, 16, 48, 32);
      gl.clearColor(1, 0, 0, 1);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.disable(gl.SCISSOR_TEST);
      await Promise.resolve();
      await new Promise((r) => requestAnimationFrame(() => r(null)));
      const layer = document.getElementById('engine-compat-context') as HTMLCanvasElement | null;
      const present = document.getElementById('engine-present') as HTMLCanvasElement | null;
      const bound = api.render.gl.canvas as HTMLCanvasElement;
      const box = (el: Element | null) => { const r = el?.getBoundingClientRect(); return r ? [r.left, r.top, r.width, r.height].map((v) => Math.round(v * 10) / 10) : null; };
      const layerPixels = layer ? Array.from(layer.getContext('2d')!.getImageData(0, 0, layer.width, layer.height).data) : null;
      out.layer = {
        shown: layer?.style.visibility === 'visible',
        abovePresentation: !!present && !!layer && present.compareDocumentPosition(layer) === Node.DOCUMENT_POSITION_FOLLOWING && layer.previousElementSibling === present,
        box: box(layer), boundBox: box(bound),
        size: layer ? [layer.width, layer.height] : null
      };
      // Captures and exports never include it (today they render their own passes).
      const before = api.render.renderFrameTo(PM.time, 320, 180, { mblur: false }).getContext('2d').getImageData(0, 0, 320, 180).data;
      out.captureHasRed = (() => { for (let p = 0; p < before.length; p += 4) if (before[p] > 240 && before[p + 1] < 15 && before[p + 2] < 15) return true; return false; })();
      // Today's compositor on the same bound canvas: the same frame, the same drawing.
      const legacy: WebGL2RenderingContext = PM.GL.gl;
      PM.GL.render(PM.time);
      const today = new Uint8Array(W * H * 4);
      legacy.readPixels(0, 0, W, H, legacy.RGBA, legacy.UNSIGNED_BYTE, today);
      legacy.enable(legacy.SCISSOR_TEST); legacy.scissor(16, 16, 48, 32); legacy.clearColor(1, 0, 0, 1); legacy.clear(legacy.COLOR_BUFFER_BIT); legacy.disable(legacy.SCISSOR_TEST);
      const todayDrawn = document.createElement('canvas'); todayDrawn.width = W; todayDrawn.height = H;
      todayDrawn.getContext('2d')!.drawImage(legacy.canvas as HTMLCanvasElement, 0, 0);
      const todayDrawnPixels = todayDrawn.getContext('2d')!.getImageData(0, 0, W, H).data;
      const compare = (a: ArrayLike<number>, b: ArrayLike<number>) => {
        let max = 0, over1 = 0, values = 0;
        for (let i = 0; i < a.length; i++) {
          if (i % 4 === 3) continue;
          const d = Math.abs(a[i]! - b[i]!);
          values++;
          if (d > max) max = d;
          if (d > 1) over1++;
        }
        return { max, over1, values };
      };
      out.mechanism = compare(flipped, onScreenPixels);
      out.readback = compare(engine, today);
      // Where the engine's frame and today's canvas differ by more than 4.
      const rows = new Map<number, number>();
      for (let i = 0; i < engine.length; i += 4) {
        if (Math.max(Math.abs(engine[i]! - today[i]!), Math.abs(engine[i + 1]! - today[i + 1]!), Math.abs(engine[i + 2]! - today[i + 2]!)) > 4) {
          const y = H - 1 - Math.floor(i / 4 / W);
          rows.set(y, (rows.get(y) ?? 0) + 1);
        }
      }
      out.overFourRows = [...rows].sort((a, b) => b[1] - a[1]).slice(0, 8);
      out.drawn = layerPixels ? compare(layerPixels, todayDrawnPixels) : null;
      const at = (px: ArrayLike<number>, x: number, y: number) => { const i = (y * W + x) * 4; return [px[i], px[i + 1], px[i + 2], px[i + 3]]; };
      // GL row 20 from the bottom is the layer's row H-1-20 from the top.
      out.redInLayer = layerPixels ? at(layerPixels, 30, H - 1 - 20) : null;
      out.alphaOpaque = Array.from(engine.filter((_, i) => i % 4 === 3)).every((a) => a === 255);
      // The next frame replaces the drawing.
      await nextFrame('after');
      out.afterNextFrame = { shown: layer?.style.visibility === 'visible' };
      out.stats = { ...PM.CompatContext.stats };
      // Contact sheet: engine readback (flipped to top-down), today's readback.
      const sheet = document.createElement('canvas'); sheet.width = W * 2; sheet.height = H;
      const sx = sheet.getContext('2d')!;
      for (const [k, px] of [[0, engine], [1, today]] as const) {
        const image = sx.createImageData(W, H);
        for (let y = 0; y < H; y++) image.data.set(px.subarray((H - 1 - y) * W * 4, (H - y) * W * 4), y * W * 4);
        sx.putImageData(image, k * W, 0);
      }
      out.sheet = sheet.toDataURL();
      return out;
    });
    const sheet = testInfo.outputPath('compat-context-engine-left-today-right.png');
    await writeFile(sheet, Buffer.from(String(result.sheet).split(',')[1]!, 'base64'));
    await testInfo.attach('compat-context-readback', { path: sheet, contentType: 'image/png' });
    delete result.sheet;
    console.log(JSON.stringify(result));
    expect(result.timeouts).toEqual([]);
    const identity = result.identity as any;
    expect(identity).toMatchObject({ isContext: true, sameObject: true, canvasIsBound: true, offscreen: true });
    expect(identity.attributes).toMatchObject({ alpha: false, antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: true });
    const size = result.size as any;
    expect([size.W, size.H]).toEqual([size.boundW, size.boundH]);
    // Reading back gives the frame on screen exactly (bottom row first, opaque).
    expect((result.mechanism as any).max).toBe(0);
    expect(result.alphaOpaque).toBe(true);
    // Drawing: shown over the engine's canvas, at the bound canvas's place, as today's canvas would show it.
    const layer = result.layer as any;
    expect(layer.shown).toBe(true);
    expect(layer.abovePresentation).toBe(true);
    expect(layer.box).toEqual(layer.boundBox);
    expect(layer.size).toEqual([size.W, size.H]);
    expect(result.redInLayer).toEqual([255, 0, 0, 255]);
    expect(result.captureHasRed).toBe(false);
    expect((result.afterNextFrame as any).shown).toBe(false);
    expect(session.diagnostics.pageErrors).toEqual([]);
    // And what it reads and shows is today's: the engine's frame against
    // today's compositor on the same canvas (section 25: <= 1 CV for 99.9%
    // of values, <= 4 for any). A difference here is the engine's frame on
    // screen, not the compatibility context (the check above).
    const readback = result.readback as any;
    expect(readback.over1, 'engine frame vs today').toBeLessThanOrEqual(readback.values * 0.001);
    expect(readback.max, 'engine frame vs today').toBeLessThanOrEqual(4);
    const drawn = result.drawn as any;
    expect(drawn.over1, 'shown drawing vs today').toBeLessThanOrEqual(drawn.values * 0.001);
    expect(drawn.max, 'shown drawing vs today').toBeLessThanOrEqual(4);
  });
});
