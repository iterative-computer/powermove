import { execFile } from 'node:child_process';
import type { Page } from 'playwright';
import { mkdir, mkdtemp, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { test, expect, repoRoot } from './helpers/app';

const exec = promisify(execFile);
// Frozen before the consumer rewrite: this viewer has no Library render callback.
const FROZEN_VIEWER = '8d772dac8';

async function installFrozenViewer(userData: string): Promise<void> {
  const prefix = 'apps/desktop/src/extensions/viewer/';
  const gitRoot = path.resolve(repoRoot, '../..');
  const { stdout } = await exec('git', ['ls-tree', '-r', '--name-only', FROZEN_VIEWER, '--', prefix], { cwd: gitRoot });
  const root = path.join(userData, 'extensions', 'consumer-viewer-fork');
  // Publish the whole fork at once; the loader must never see a manifest
  // while its source tree is only partly copied.
  const staging = await mkdtemp(path.join(userData, 'consumer-viewer-'));
  try {
    for (const file of stdout.trim().split('\n').filter(file => !file.endsWith('.test.ts'))) {
      const { stdout: source } = await exec('git', ['show', `${FROZEN_VIEWER}:${file}`], { cwd: gitRoot, maxBuffer: 4 * 1024 * 1024 });
      const destination = path.join(staging, file.slice(prefix.length));
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, source);
    }
    const { stdout: manifest } = await exec('git', ['show', `${FROZEN_VIEWER}:${prefix}manifest.json`], { cwd: gitRoot });
    const parsed = JSON.parse(manifest);
    await writeFile(path.join(staging, 'manifest.json'), JSON.stringify({ ...parsed, id: 'consumer-viewer-fork', replaces: ['viewer'], forkedFrom: `viewer@${parsed.version}` }));
    await mkdir(path.dirname(root), { recursive: true });
    await rename(staging, root);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

async function clearViewerHover(page: Page): Promise<void> {
  const outside = await page.evaluate(() => {
    const stage = document.querySelector('#panel-viewer #stage')!.getBoundingClientRect();
    const candidates: Array<[number, number]> = [[1, 1], [innerWidth - 2, 1], [1, innerHeight - 2], [innerWidth - 2, innerHeight - 2]];
    const point = candidates.find(([x, y]) => x < stage.left || x > stage.right || y < stage.top || y > stage.bottom);
    if (!point) throw new Error('no pointer position outside the viewer stage');
    return { x: point[0]!, y: point[1]! };
  });
  await page.mouse.move(outside.x, outside.y);
}

type Box = { left: number; top: number; width: number; height: number };
const COMP = { w: 320, h: 180 };
// The selected layer: a 120×80 solid at (40,30)–(160,110), on black.
const LAYER = { left: 40, top: 30, right: 160, bottom: 110 };
const ZOOM = 2;
const PAN: [number, number] = [-30, 16];

for (const theme of ['light', 'dark']) for (const frozen of [false, true]) {
  test(`Library viewer card at 200% with a selection in ${theme}, ${frozen ? 'frozen fork' : 'current viewer'}`, async ({ session }, testInfo) => {
    await session.openEditor();
    const { page } = session;
    const engine = await page.evaluate(() => !!(window as any).PM.EngineRender);
    if (frozen) {
      await installFrozenViewer(session.userData);
      await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.loader.activeIds().includes('consumer-viewer-fork')), { timeout: 30_000 }).toBe(true);
    }
    await page.evaluate(({ theme, zoom, pan, comp, layer }) => {
      const PM = (window as any).PM;
      PM.theme.apply(theme);
      const project = PM.mkProject({ name: 'Consumer surfaces', w: comp.w, h: comp.h, fps: 30, dur: 2, bg: '#000000' });
      const w = layer.right - layer.left, h = layer.bottom - layer.top;
      const solid = PM.mkLayer('solid', { from: 0, dur: 2, d: { w, h, color: '#2040E0' } }, project);
      // Solids have a zero anchor, and mkLayer resets their initial position.
      solid.p['position.x'].v = layer.left;
      solid.p['position.y'].v = layer.top;
      project.layers = [solid];
      PM.replaceProject(project); PM.setTime(.5, { force: true });
      PM.selectLayers(solid.id);
      const viewer = PM.Kernel.services.get('viewer');
      viewer.setZoom(zoom);
      viewer.pan = [...pan];
      viewer.layout();
      PM.invalidate();
    }, { theme, zoom: ZOOM, pan: PAN, comp: COMP, layer: LAYER });

    /* Wait for the synthetic pixels and selected outline to settle together.
       A visible presentation canvas may still hold the previous frame. */
    await clearViewerHover(page);
    await page.evaluate(() => document.fonts.ready);
    const stableViewer = () => page.evaluate(async ({ engine, layer, zoom }) => {
      const panel = (window as any).PM.panelInst.viewer.el as HTMLElement;
      const stage = panel.querySelector<HTMLElement>('#stage')!;
      const inner = panel.querySelector<HTMLElement>('#stage-inner')!;
      const overlay = panel.querySelector<HTMLCanvasElement>('#overlay')!;
      const sample = () => {
        const data = overlay.getContext('2d')!.getImageData(0, 0, overlay.width, overlay.height).data;
        let hash = 2166136261, visible = false;
        for (let index = 0; index < data.length; index++) {
          hash = Math.imul(hash ^ data[index]!, 16777619) >>> 0;
          if (index % 4 === 3 && data[index]) visible = true;
        }
        if (!engine) return { hash, visible, picture: true };
        const native = stage.querySelector<HTMLCanvasElement>('#engine-present');
        if (!native || getComputedStyle(native).visibility !== 'visible') return { hash, visible, picture: false };
        const copy = document.createElement('canvas'); copy.width = native.width; copy.height = native.height;
        const ctx = copy.getContext('2d')!;
        ctx.drawImage(native, 0, 0);
        const css = native.getBoundingClientRect();
        const pixel = (x: number, y: number) => {
          const sx = Math.floor((parseFloat(inner.style.left) + x * zoom) * copy.width / css.width);
          const sy = Math.floor((parseFloat(inner.style.top) + y * zoom) * copy.height / css.height);
          return Array.from(ctx.getImageData(sx, sy, 1, 1).data);
        };
        const inside = pixel(layer.left + 3, layer.top + 20);
        const outside = pixel(layer.left - 3, layer.top + 20);
        const center = pixel((layer.left + layer.right) / 2, (layer.top + layer.bottom) / 2);
        return { hash, visible, picture: inside[2]! > 180 && inside[0]! < 80
          && outside[0]! + outside[1]! + outside[2]! < 60 && center[2]! > 180 };
      };
      const first = sample();
      for (let frame = 0; frame < 3; frame++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      const last = sample();
      return first.visible && first.picture && last.visible && last.picture && first.hash === last.hash;
    }, { engine, layer: LAYER, zoom: ZOOM });
    await expect.poll(stableViewer, { timeout: 10_000 }).toBe(true);

    /* The live viewer's geometry, overlay, and native bitmap from its DOM. */
    const live = () => page.evaluate(() => {
      const PM = (window as any).PM;
      const panel = PM.panelInst.viewer.el as HTMLElement;
      const inner = panel.querySelector<HTMLElement>('#stage-inner')!;
      const overlay = panel.querySelector<HTMLCanvasElement>('#overlay')!;
      const hashCanvas = (canvas: HTMLCanvasElement) => {
        const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
        let hash = 2166136261;
        for (let index = 0; index < data.length; index++) hash = Math.imul(hash ^ data[index]!, 16777619) >>> 0;
        return hash;
      };
      const native = panel.querySelector<HTMLCanvasElement>('#engine-present');
      let present = null;
      if (native && getComputedStyle(native).visibility === 'visible') {
        const bitmap = document.createElement('canvas'); bitmap.width = native.width; bitmap.height = native.height;
        bitmap.getContext('2d')!.drawImage(native, 0, 0);
        const css = getComputedStyle(native);
        present = { width: native.width, height: native.height, css: [css.width, css.height], hash: hashCanvas(bitmap) };
      }
      return {
        inner: [inner.style.left, inner.style.top, inner.style.width, inner.style.height],
        overlay: { width: overlay.width, height: overlay.height, css: [overlay.style.width, overlay.style.height], parent: overlay.parentElement?.id, hash: hashCanvas(overlay) },
        present,
        zoom: PM.Kernel.services.get('viewer').shown,
      };
    });
    if (engine) await expect(page.locator('#panel-viewer #engine-present')).toHaveCSS('visibility', 'visible');
    const liveArtifact = testInfo.outputPath('viewer-live.png');
    await page.locator('#panel-viewer #stage').screenshot({ path: liveArtifact });
    await testInfo.attach('Live viewer', { path: liveArtifact, contentType: 'image/png' });
    if (engine) {
      // Record whether a paused presentation canvas can be cloned directly.
      // The readback is diagnostic; the card assertions below remain unchanged.
      const readback = await page.evaluate(() => {
        const source = document.querySelector<HTMLCanvasElement>('#panel-viewer #engine-present')!;
        const copy = document.createElement('canvas'); copy.width = source.width; copy.height = source.height;
        try {
          copy.getContext('2d')!.drawImage(source, 0, 0);
          return { png: copy.toDataURL(), error: null };
        } catch (error) { return { png: null, error: String(error) }; }
      });
      if (readback.png) await testInfo.attach('Native canvas copy', { body: Buffer.from(readback.png.split(',')[1]!, 'base64'), contentType: 'image/png' });
      else await testInfo.attach('Native canvas copy error', { body: readback.error ?? 'No readback', contentType: 'text/plain' });
    }
    const before = await live();
    expect(before.zoom).toBe(ZOOM);
    expect(before.inner[2]).toBe(`${COMP.w * ZOOM}px`);
    expect(before.inner[3]).toBe(`${COMP.h * ZOOM}px`);

    await page.evaluate(() => (window as any).PM.LibraryUI.open());
    const card = page.locator('.library-card[data-panel-id="viewer"]');
    await card.scrollIntoViewIfNeeded();
    await expect(card.locator('.library-clone')).toBeAttached();
    await expect(card.locator('[data-library-engine-frame]')).toHaveCount(0);

    // Compare the same non-hover state after the Library has loaded its fonts.
    await clearViewerHover(page);
    await page.evaluate(() => document.fonts.ready);
    await expect.poll(stableViewer, { timeout: 10_000 }).toBe(true);
    // The live viewer kept its zoom, pan and overlay while the Library opened.
    const after = await live();
    expect(after).toEqual(before);

    const clone = await card.evaluate((cardElement, { layer, zoom }) => {
      const root = cardElement.querySelector<HTMLElement>('.library-clone')!;
      const stage = root.querySelector<HTMLElement>('[data-library-source-id="stage"]')!;
      const inner = root.querySelector<HTMLElement>('[data-library-source-id="stage-inner"]')!;
      const overlay = root.querySelector<HTMLCanvasElement>('[data-library-source-id="overlay"]')!;
      const present = root.querySelector<HTMLCanvasElement>('canvas[data-library-source-id="engine-present"]');
      const data = overlay.getContext('2d')!.getImageData(0, 0, overlay.width, overlay.height).data;
      let hash = 2166136261;
      for (let index = 0; index < data.length; index++) hash = Math.imul(hash ^ data[index]!, 16777619) >>> 0;
      const box = (element: Element) => { const r = element.getBoundingClientRect(); return { left: r.left, top: r.top, width: r.width, height: r.height }; };
      /* The outline on the copied overlay: the first painted pixel within
         3 CSS px of the layer's left edge, on a row clear of the handles. */
      const row = (layer.top + 20) * zoom + parseFloat(inner.style.top);
      const edge = layer.left * zoom + parseFloat(inner.style.left);
      const density = overlay.width / parseFloat(overlay.style.width);
      const rowData = overlay.getContext('2d')!.getImageData(0, Math.round(row * density), overlay.width, 1).data;
      let outline: number | null = null;
      for (let x = Math.floor((edge - 3) * density); x <= Math.ceil((edge + 3) * density); x++) {
        if (x >= 0 && x < overlay.width && rowData[x * 4 + 3]! > 0) { outline = x / density; break; }
      }
      const pixel = (canvas: HTMLCanvasElement, compX: number, compY: number) => {
        const cssWidth = parseFloat(canvas.style.width), cssHeight = parseFloat(canvas.style.height);
        const x = Math.floor((parseFloat(inner.style.left) + compX * zoom) * canvas.width / cssWidth);
        const y = Math.floor((parseFloat(inner.style.top) + compY * zoom) * canvas.height / cssHeight);
        return Array.from(canvas.getContext('2d')!.getImageData(x, y, 1, 1).data);
      };
      const hashCanvas = (canvas: HTMLCanvasElement) => {
        const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
        let value = 2166136261;
        for (let index = 0; index < pixels.length; index++) value = Math.imul(value ^ pixels[index]!, 16777619) >>> 0;
        return value;
      };
      return {
        inner: [inner.style.left, inner.style.top, inner.style.width, inner.style.height],
        overlay: { width: overlay.width, height: overlay.height, css: [overlay.style.width, overlay.style.height], parent: overlay.parentElement?.dataset.librarySourceId, hash },
        stage: box(stage), innerBox: box(inner), frameBox: box(cardElement.querySelector('.library-live-frame')!),
        presentBox: present && box(present),
        present: present && {
          width: present.width, height: present.height,
          css: [present.style.width, present.style.height],
          hash: hashCanvas(present),
          visibility: getComputedStyle(present).visibility,
          inside: pixel(present, layer.left + 3, layer.top + 20),
          outside: pixel(present, layer.left - 3, layer.top + 20),
          center: pixel(present, (layer.left + layer.right) / 2, (layer.top + layer.bottom) / 2),
        },
        edge, outline,
      };
    }, { layer: LAYER, zoom: ZOOM });

    // No refit: the clone's stage-inner is the live one, and the copied overlay is the live overlay, pixel for pixel.
    expect(clone.inner).toEqual(after.inner);
    expect(clone.overlay).toEqual({ ...after.overlay, parent: 'stage' });
    expect(after.overlay.parent).toBe('stage');
    // The selection outline sits on the layer's left edge in stage coordinates.
    expect(clone.outline).not.toBeNull();
    expect(Math.abs(clone.outline! - clone.edge)).toBeLessThanOrEqual(3);
    if (engine) {
      const present = clone.present!;
      expect(present).not.toBeNull();
      expect(present.visibility).toBe('visible');
      expect({ width: present.width, height: present.height, css: present.css, hash: present.hash }).toEqual(after.present);
      // Stage-level presentation pixels retain the source stage's CSS box,
      // even though the Library card itself has a different fixed size.
      expect(clone.presentBox!.left).toBeCloseTo(clone.stage.left, 1);
      expect(clone.presentBox!.top).toBeCloseTo(clone.stage.top, 1);
      const cardScale = clone.innerBox.width / (COMP.w * ZOOM);
      expect(clone.presentBox!.width).toBeCloseTo(parseFloat(after.present!.css[0]) * cardScale, 1);
      expect(clone.presentBox!.height).toBeCloseTo(parseFloat(after.present!.css[1]) * cardScale, 1);
      // Picture and selected outline agree at the live zoom and pan.
      expect(present.inside[2]).toBeGreaterThan(180);
      expect(present.inside[0]).toBeLessThan(80);
      expect(present.outside[0] + present.outside[1] + present.outside[2]).toBeLessThan(60);
      expect(present.center[2]).toBeGreaterThan(180);
      // The layer is visible in the card, not cropped away.
      const scale = clone.innerBox.width / (COMP.w * ZOOM);
      const point = { x: clone.innerBox.left + (LAYER.left + LAYER.right) / 2 * ZOOM * scale, y: clone.innerBox.top + (LAYER.top + LAYER.bottom) / 2 * ZOOM * scale };
      for (const bounds of [clone.stage, clone.frameBox] as Box[]) {
        expect(point.x).toBeGreaterThan(bounds.left);
        expect(point.x).toBeLessThan(bounds.left + bounds.width);
        expect(point.y).toBeGreaterThan(bounds.top);
        expect(point.y).toBeLessThan(bounds.top + bounds.height);
      }
    }
    const artifact = testInfo.outputPath('library.png');
    await card.screenshot({ path: artifact });
    await testInfo.attach('Library viewer card', { path: artifact, contentType: 'image/png' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
