import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { test, expect, repoRoot } from './helpers/app';

const exec = promisify(execFile);
// Frozen before the consumer rewrite: this viewer has no Library render callback.
const FROZEN_VIEWER = '8d772dac8';

async function installFrozenViewer(userData: string): Promise<void> {
  const prefix = 'apps/desktop/src/extensions/viewer/';
  const { stdout } = await exec('git', ['ls-tree', '-r', '--name-only', FROZEN_VIEWER, '--', prefix], { cwd: repoRoot });
  const root = path.join(userData, 'extensions', 'consumer-viewer-fork');
  // Publish the whole fork at once; the loader must never see a manifest
  // while its source tree is only partly copied.
  const staging = await mkdtemp(path.join(userData, 'consumer-viewer-'));
  try {
    for (const file of stdout.trim().split('\n').filter(file => !file.endsWith('.test.ts'))) {
      const { stdout: source } = await exec('git', ['show', `${FROZEN_VIEWER}:${file}`], { cwd: repoRoot, maxBuffer: 4 * 1024 * 1024 });
      const destination = path.join(staging, file.slice(prefix.length));
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, source);
    }
    const { stdout: manifest } = await exec('git', ['show', `${FROZEN_VIEWER}:${prefix}manifest.json`], { cwd: repoRoot });
    const parsed = JSON.parse(manifest);
    await writeFile(path.join(staging, 'manifest.json'), JSON.stringify({ ...parsed, id: 'consumer-viewer-fork', replaces: ['viewer'], forkedFrom: `viewer@${parsed.version}` }));
    await mkdir(path.dirname(root), { recursive: true });
    await rename(staging, root);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
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
      const solid = PM.mkLayer('solid', { from: 0, dur: 2, d: { w, h, color: '#2040E0' }, p: { 'position.x': layer.left + w / 2, 'position.y': layer.top + h / 2 } }, project);
      project.layers = [solid];
      PM.replaceProject(project); PM.setTime(.5, { force: true });
      PM.selectLayers(solid.id);
      const viewer = PM.Kernel.services.get('viewer');
      viewer.setZoom(zoom);
      viewer.pan = [...pan];
      viewer.layout();
      PM.invalidate();
    }, { theme, zoom: ZOOM, pan: PAN, comp: COMP, layer: LAYER });

    /* The live viewer's geometry and overlay, read from its own DOM. */
    const live = () => page.evaluate(() => {
      const PM = (window as any).PM;
      const panel = PM.panelInst.viewer.el as HTMLElement;
      const inner = panel.querySelector<HTMLElement>('#stage-inner')!;
      const overlay = panel.querySelector<HTMLCanvasElement>('#overlay')!;
      const data = overlay.getContext('2d')!.getImageData(0, 0, overlay.width, overlay.height).data;
      let hash = 2166136261;
      for (let index = 0; index < data.length; index++) hash = Math.imul(hash ^ data[index]!, 16777619) >>> 0;
      return {
        inner: [inner.style.left, inner.style.top, inner.style.width, inner.style.height],
        overlay: { width: overlay.width, height: overlay.height, css: [overlay.style.width, overlay.style.height], parent: overlay.parentElement?.id, hash },
        zoom: PM.Kernel.services.get('viewer').shown,
      };
    });
    // The selection outline is on the live overlay before the Library opens.
    await expect.poll(() => page.evaluate(() => {
      const overlay = (window as any).PM.panelInst.viewer.el.querySelector('#overlay') as HTMLCanvasElement;
      const data = overlay.getContext('2d')!.getImageData(0, 0, overlay.width, overlay.height).data;
      for (let index = 3; index < data.length; index += 4) if (data[index]) return true;
      return false;
    }), { timeout: 10_000 }).toBe(true);
    const before = await live();
    expect(before.zoom).toBe(ZOOM);
    expect(before.inner[2]).toBe(`${COMP.w * ZOOM}px`);
    expect(before.inner[3]).toBe(`${COMP.h * ZOOM}px`);

    await page.evaluate(() => (window as any).PM.LibraryUI.open());
    const card = page.locator('.library-card[data-panel-id="viewer"]');
    await card.scrollIntoViewIfNeeded();
    await expect(card.locator('.library-clone')).toBeAttached();
    if (engine) await expect(card.locator('[data-library-source-id="stage-inner"] > canvas[data-library-engine-frame]')).toBeAttached({ timeout: 30_000 });
    else await expect(card.locator('[data-library-engine-frame]')).toHaveCount(0);

    // The live viewer kept its zoom, pan and overlay while the Library opened.
    const after = await live();
    expect(after).toEqual(before);

    const clone = await card.evaluate((cardElement, { comp, layer, zoom }) => {
      const root = cardElement.querySelector<HTMLElement>('.library-clone')!;
      const stage = root.querySelector<HTMLElement>('[data-library-source-id="stage"]')!;
      const inner = root.querySelector<HTMLElement>('[data-library-source-id="stage-inner"]')!;
      const overlay = root.querySelector<HTMLCanvasElement>('[data-library-source-id="overlay"]')!;
      const cover = inner.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-engine-frame]');
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
      const pixel = (canvas: HTMLCanvasElement, compX: number, compY: number) => Array.from(canvas.getContext('2d')!
        .getImageData(Math.floor(compX / comp.w * canvas.width), Math.floor(compY / comp.h * canvas.height), 1, 1).data);
      return {
        inner: [inner.style.left, inner.style.top, inner.style.width, inner.style.height],
        overlay: { width: overlay.width, height: overlay.height, css: [overlay.style.width, overlay.style.height], parent: overlay.parentElement?.dataset.librarySourceId, hash },
        stage: box(stage), innerBox: box(inner), frameBox: box(cardElement.querySelector('.library-live-frame')!),
        coverBox: cover && box(cover),
        present: present && getComputedStyle(present).visibility,
        cover: cover && {
          style: [cover.style.position, cover.style.left, cover.style.top, cover.style.width, cover.style.height],
          aspect: cover.width / cover.height,
          size: [cover.width, cover.height],
          inside: pixel(cover, layer.left + 3, layer.top + 20),
          outside: pixel(cover, layer.left - 3, layer.top + 20),
          center: pixel(cover, (layer.left + layer.right) / 2, (layer.top + layer.bottom) / 2),
        },
        edge, outline,
      };
    }, { comp: COMP, layer: LAYER, zoom: ZOOM });

    // No refit: the clone's stage-inner is the live one, and the copied overlay is the live overlay, pixel for pixel.
    expect(clone.inner).toEqual(after.inner);
    expect(clone.overlay).toEqual({ ...after.overlay, parent: 'stage' });
    expect(after.overlay.parent).toBe('stage');
    // The selection outline sits on the layer's left edge in stage coordinates.
    expect(clone.outline).not.toBeNull();
    expect(Math.abs(clone.outline! - clone.edge)).toBeLessThanOrEqual(3);
    if (engine) {
      const cover = clone.cover!;
      expect(cover.style).toEqual(['absolute', '0px', '0px', '100%', '100%']);
      expect(cover.aspect).toBeCloseTo(COMP.w / COMP.h, 2);
      // Rendered at the live viewer's size: never above the composition at 200%, not the zoomed CSS box × density.
      expect(cover.size[0]).toBeLessThanOrEqual(COMP.w);
      expect(cover.size[1]).toBeLessThanOrEqual(COMP.h);
      // A cloned native presentation canvas would sit above the cover.
      if (clone.present !== null) expect(clone.present).toBe('hidden');
      // The cover is exactly the clone's stage-inner box: the full composition at the live zoom and pan.
      for (const key of ['left', 'top', 'width', 'height'] as const) expect(clone.coverBox![key]).toBeCloseTo(clone.innerBox[key], 1);
      // Picture and outline agree: blue just inside the outlined edge, black just outside it.
      expect(cover.inside[2]).toBeGreaterThan(180);
      expect(cover.inside[0]).toBeLessThan(80);
      expect(cover.outside[0] + cover.outside[1] + cover.outside[2]).toBeLessThan(60);
      expect(cover.center[2]).toBeGreaterThan(180);
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
