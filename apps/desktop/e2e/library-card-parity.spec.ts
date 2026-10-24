/*
 * Library viewer cards with "Use New Engine" on are today's cards, pixel for
 * pixel (plan appendix C row C3; owner rule: no visible difference on an
 * existing project). Each case launches the app twice, engine off then on,
 * one Electron at a time, builds the same scene with the same viewer, opens
 * the Library, and compares the two card images exactly. 3D cards use the
 * reference rule (QUALITY-1): pixels away from edges are exact, and an edge
 * pixel may differ only if it is at least as close as today's to the
 * engine's supersampled render of the same pinned view.
 *
 * Run serially: pm-electron-lock bunx playwright test e2e/library-card-parity.spec.ts --workers=1
 */
import { writeFile } from 'node:fs/promises';
import type { Locator, Page } from 'playwright';

import { expect, launchApp, test } from './helpers/app';
import { VIEWER_VARIANTS, useViewer, type ViewerVariant } from './helpers/viewer-fork';

/** 2D at 200% panned, 2D at Fit, a 3D model through its camera, and in the 3D Editor view framed on it. */
type Scene = '2d' | '2d-fit' | '3d' | '3d-editor';

async function card<T = Buffer>(engine: '0' | '1', theme: string, variant: ViewerVariant, scene: Scene, then?: (page: Page, png: Buffer) => Promise<T>): Promise<T> {
  // Match engine-viewer-canvas's Standard-color gate. The display-profile
  // diagnostic also compares the complete card exactly; CSS capture
  // differences remain failures (docs/engine/consumers.md).
  const args = process.env.PM_LIBRARY_CARD_PROFILE === 'display' ? [] : ['--force-color-profile=srgb'];
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: engine }, args });
  try {
    await session.openEditor();
    const { page } = session;
    expect(await page.evaluate(() => !!(window as any).PM.EngineRender)).toBe(engine === '1');
    await useViewer(page, session.userData, variant);
    await page.evaluate(({ theme, scene }) => {
      const PM = (window as any).PM;
      PM.theme.apply(theme);
      if (scene === '2d' || scene === '2d-fit') {
        const project = PM.mkProject({ name: 'Card parity', w: 320, h: 180, fps: 30, dur: 2, bg: '#000000' });
        const solid = PM.mkLayer('solid', { from: 0, dur: 2, d: { w: 120, h: 80, color: '#2040E0' } }, project);
        solid.p['position.x'].v = 40; solid.p['position.y'].v = 30;
        project.layers = [solid];
        PM.replaceProject(project); PM.setTime(.5, { force: true });
        PM.selectLayers(solid.id);
        const viewer = PM.Kernel.services.get('viewer');
        if (scene === '2d') { viewer.setZoom(2); viewer.pan = [-30, 16]; viewer.layout(); }
      } else {
        const api = PM.Kernel.api('card-parity');
        api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 1.5, z: 7 } } });
        api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'point', p: { x: -2, y: 3, z: 2, intensity: 30 } } });
        api.scene3d.edit({ operation: 'add_object', object: { id: 'cube', source: { primitive: 'box' }, material: { p: { color: '#ff7733' } } } });
        PM.selectLayers(['cube']);
        if (scene === '3d-editor') { api.scene3d.setView('editor'); api.scene3d.frame(true); }
      }
      PM.invalidate();
    }, { theme, scene });
    await settle(page);
    await page.evaluate(() => (window as any).PM.LibraryUI.open());
    // Match the grid and scroll-raster coordinates: unrelated live previews
    // can have different heights. Use the Library's existing search UI.
    const search = page.getByRole('searchbox', { name: 'Search panels', exact: true });
    await search.fill('viewer');
    await expect(search).toHaveValue('viewer');
    await expect(page.locator('.library-card[data-panel-id]')).toHaveCount(1);
    const shown = page.locator('.library-card[data-panel-id="viewer"]');
    await shown.scrollIntoViewIfNeeded();
    await expect(shown.locator('.library-clone')).toBeAttached();
    await expect(page.locator('[data-previews="ready"]')).toBeAttached();
    await page.evaluate(() => document.fonts.ready);
    // Async native cards and font loading can change the shelf's height.
    // Align the final card, so both captures use the same device-pixel origin
    // (including the card's gradient and rounded corners).
    await shown.evaluate(element => element.scrollIntoView({ block: 'start', inline: 'nearest', behavior: 'instant' }));
    await page.mouse.move(1, 1);
    await shown.evaluate(element => Promise.all(element.getAnimations({ subtree: true })
      .filter(animation => Number.isFinite(animation.effect?.getComputedTiming().endTime))
      .map(animation => animation.finished.catch(() => undefined))));
    const png = await freshCapture(shown);
    return then ? await then(page, png) : png as T;
  } finally {
    await session.close();
  }
}

/**
 * Chromium's tiles keep their raster history: after partial re-rasters, the
 * same final DOM can differ across launches at a few antialiased SVG samples
 * (OFF/OFF launches differed by up to 36 levels on header icons), and the
 * shelf ledge's dithered gradient behind the label by up to 2. Recreating the
 * clone's layers and the ledge rasterizes that final DOM in one pass, so every
 * capture compares the same content, not each launch's history.
 */
async function freshCapture(shown: Locator): Promise<Buffer> {
  await shown.evaluate(element => new Promise<void>(resolve => {
    const clone = element.querySelector<HTMLElement>('.library-clone')!;
    const display = clone.style.display;
    const ledge = element.ownerDocument.createElement('style');
    ledge.textContent = '.library-shelf-row::before{display:none}';
    clone.style.display = 'none';
    element.ownerDocument.head.append(ledge);
    requestAnimationFrame(() => {
      clone.style.display = display;
      ledge.remove();
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
  }));
  return shown.screenshot({ animations: 'disabled' });
}

/**
 * The same card with its composition from the engine's 4× supersampled render
 * of the pinned view (16 pixels, each 4× MSAA, box-averaged per card pixel).
 */
async function referenceCard(page: Page): Promise<Buffer> {
  await page.evaluate(async () => {
    const PM = (window as any).PM, k = 4;
    const presentation = PM.Kernel.services.get('viewer')?.enginePresentation ?? PM.EngineCanvas?.viewerPresentation;
    const state = presentation.stateFor(presentation.latest.viewSeq);
    const { width, height } = state.canvas;
    const snapshot = await PM.EngineRender.thumbnails.request({
      key: 'c3-reference', composition: String(PM.proj.compId ?? PM.proj.id ?? ''), time: PM.time,
      width: width * k, height: height * k, pixels: 'opaque', draft3d: true,
      viewer: { view: state.view ?? null, canvas: { ...state.canvas, width: width * k, height: height * k }, viewSeq: state.viewSeq },
    });
    const big = PM.EngineRender.toCanvas(snapshot).getContext('2d').getImageData(0, 0, width * k, height * k).data;
    const out = new ImageData(width, height);
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) for (let c = 0; c < 4; c++) {
      let sum = 0;
      for (let j = 0; j < k; j++) for (let i = 0; i < k; i++) sum += big[(((y * k + j) * width * k) + x * k + i) * 4 + c]!;
      out.data[(y * width + x) * 4 + c] = Math.round(sum / (k * k));
    }
    const target = [...document.querySelectorAll<HTMLCanvasElement>('.library-card[data-panel-id="viewer"] .library-clone canvas')]
      .find(canvas => !canvas.hidden && canvas.width === width && canvas.height === height);
    if (!target) throw new Error('No cloned composition canvas at the pinned backing size');
    target.getContext('2d')!.putImageData(out, 0, 0);
  });
  return freshCapture(page.locator('.library-card[data-panel-id="viewer"]'));
}

/** The viewer has drawn this scene: two samples a few frames apart agree. */
async function settle(page: Page): Promise<void> {
  await page.mouse.move(1, 1);
  // With the engine on, its frames are on screen (no "Starting the engine…").
  if (await page.evaluate(() => !!(window as any).PM.EngineRender)) {
    await expect(page.locator('#panel-viewer #engine-present')).toHaveCSS('visibility', 'visible', { timeout: 20_000 });
    await expect(page.locator('#panel-viewer [data-engine-status]')).toHaveCount(0, { timeout: 20_000 });
  }
  const sample = () => page.evaluate(async () => {
    const stage = document.querySelector('#panel-viewer #stage') as HTMLElement;
    const read = () => [...stage.querySelectorAll('canvas')].map((canvas) => {
      if (getComputedStyle(canvas).visibility !== 'visible' || !canvas.width) return 0;
      const copy = document.createElement('canvas'); copy.width = canvas.width; copy.height = canvas.height;
      const context = copy.getContext('2d')!; context.drawImage(canvas, 0, 0);
      const data = context.getImageData(0, 0, copy.width, copy.height).data;
      let hash = 2166136261;
      for (let index = 0; index < data.length; index += 7) hash = Math.imul(hash ^ data[index]!, 16777619) >>> 0;
      return hash;
    }).join(',');
    const first = read();
    for (let frame = 0; frame < 6; frame++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    return first === read() ? first : null;
  });
  await expect.poll(sample, { timeout: 15_000 }).not.toBeNull();
}

/** Compares two PNGs in a page: size, differing pixels, largest channel error. */
async function compare(page: Page, a: Buffer, b: Buffer) {
  return page.evaluate(async ({ a, b }) => {
    const load = async (data: string) => { const img = new Image(); img.src = `data:image/png;base64,${data}`; await img.decode(); return img; };
    const [left, right] = [await load(a), await load(b)];
    if (left.naturalWidth !== right.naturalWidth || left.naturalHeight !== right.naturalHeight) return { size: false, differing: -1, max: -1 };
    const pixels = (img: HTMLImageElement) => {
      const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
      const context = canvas.getContext('2d')!; context.drawImage(img, 0, 0);
      return context.getImageData(0, 0, canvas.width, canvas.height).data;
    };
    const [p, q] = [pixels(left), pixels(right)];
    let differing = 0, max = 0;
    for (let index = 0; index < p.length; index += 4) {
      let pixel = 0;
      for (let channel = 0; channel < 4; channel++) pixel = Math.max(pixel, Math.abs(p[index + channel]! - q[index + channel]!));
      if (pixel) differing++;
      max = Math.max(max, pixel);
    }
    return { size: true, differing, max };
  }, { a: a.toString('base64'), b: b.toString('base64') });
}

/**
 * The reference rule: every differing pixel must be an edge (its 3×3
 * neighbourhood in today's card spans more than 8 levels) and, per channel,
 * at least as close to the supersampled reference as today's pixel.
 */
async function compareToReference(page: Page, today: Buffer, engine: Buffer, reference: Buffer) {
  return page.evaluate(async ({ a, b, r }) => {
    const load = async (data: string) => { const img = new Image(); img.src = `data:image/png;base64,${data}`; await img.decode(); return img; };
    const images = [await load(a), await load(b), await load(r)];
    const [w, h] = [images[0]!.naturalWidth, images[0]!.naturalHeight];
    if (images.some(img => img.naturalWidth !== w || img.naturalHeight !== h)) return { size: false, interior: -1, edgeWorse: -1 };
    const pixels = (img: HTMLImageElement) => {
      const canvas = document.createElement('canvas'); canvas.width = w; canvas.height = h;
      const context = canvas.getContext('2d')!; context.drawImage(img, 0, 0);
      return context.getImageData(0, 0, w, h).data;
    };
    const [t, e, ref] = images.map(pixels) as [Uint8ClampedArray, Uint8ClampedArray, Uint8ClampedArray];
    let interior = 0, edge = 0, edgeWorse = 0;
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      if (t[i] === e[i] && t[i + 1] === e[i + 1] && t[i + 2] === e[i + 2] && t[i + 3] === e[i + 3]) continue;
      let span = 0;
      for (let c = 0; c < 3; c++) {
        let lo = 255, hi = 0;
        for (let j = Math.max(0, y - 1); j <= Math.min(h - 1, y + 1); j++) for (let k = Math.max(0, x - 1); k <= Math.min(w - 1, x + 1); k++) {
          const v = t[(j * w + k) * 4 + c]!; lo = Math.min(lo, v); hi = Math.max(hi, v);
        }
        span = Math.max(span, hi - lo);
      }
      if (span <= 8) { interior++; continue; }
      edge++;
      for (let c = 0; c < 4; c++) if (Math.abs(e[i + c]! - ref[i + c]!) > Math.abs(t[i + c]! - ref[i + c]!)) { edgeWorse++; break; }
    }
    return { size: true, interior, edgeWorse, edge };
  }, { a: today.toString('base64'), b: engine.toString('base64'), r: reference.toString('base64') });
}

const cases: Array<{ theme: string; variant: ViewerVariant; scene: Scene }> = [
  ...['light', 'dark'].flatMap(theme => (['2d', '2d-fit', '3d', '3d-editor'] as Scene[])
    .flatMap(scene => VIEWER_VARIANTS.map(variant => ({ theme, variant, scene })))),
];

async function geometry(page: Page) {
  return page.locator('.library-card[data-panel-id="viewer"]').evaluate(card => {
    const box = (element: Element) => {
      const { x, y, width, height } = element.getBoundingClientRect();
      const css = getComputedStyle(element);
      return { x, y, width, height, transform: css.transform, mask: css.maskImage, opacity: css.opacity, background: css.backgroundColor };
    };
    return [...card.querySelectorAll('.library-live,.library-live-frame,.library-clone,canvas')].map(element => ({
      id: (element as HTMLElement).dataset.librarySourceId, className: element.className, box: box(element),
      ...(element instanceof HTMLCanvasElement ? { backing: [element.width, element.height] } : {}),
    }));
  });
}

for (const { theme, variant, scene } of cases) {
  test(`Library card with the engine on is today's card: ${scene} in ${theme}, ${variant.name}`, async ({}, testInfo) => {
    test.setTimeout(120_000);
    const today = await card('0', theme, variant, scene, async (page, png) => {
      await testInfo.attach('card geometry, engine off', { body: JSON.stringify(await geometry(page)), contentType: 'application/json' });
      return png;
    });
    await writeFile(testInfo.outputPath('card-engine-off.png'), today);
    await testInfo.attach('Library card, engine off', { path: testInfo.outputPath('card-engine-off.png'), contentType: 'image/png' });
    const result = await card('1', theme, variant, scene, async (page, engine) => {
      await testInfo.attach('card geometry, engine on', { body: JSON.stringify(await geometry(page)), contentType: 'application/json' });
      await writeFile(testInfo.outputPath('card-engine-on.png'), engine);
      await testInfo.attach('Library card, engine on', { path: testInfo.outputPath('card-engine-on.png'), contentType: 'image/png' });
      if (scene === '2d' || scene === '2d-fit') return compare(page, today, engine);
      const reference = await referenceCard(page);
      await writeFile(testInfo.outputPath('card-reference.png'), reference);
      await testInfo.attach('Library card, supersampled reference', { path: testInfo.outputPath('card-reference.png'), contentType: 'image/png' });
      return compareToReference(page, today, engine, reference);
    });
    await testInfo.attach('comparison', { body: JSON.stringify(result), contentType: 'application/json' });
    if (scene === '2d' || scene === '2d-fit') expect(result).toEqual({ size: true, differing: 0, max: 0 });
    else expect(result).toMatchObject({ size: true, interior: 0, edgeWorse: 0 });
  });
}
