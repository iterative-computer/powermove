import type { Page } from 'playwright';
import { test, expect } from './helpers/app';
import { VIEWER_VARIANTS, useViewer } from './helpers/viewer-fork';

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

for (const theme of ['light', 'dark']) for (const variant of VIEWER_VARIANTS) {
  test(`Library viewer card at 200% with a selection in ${theme}, ${variant.name}`, async ({ session }, testInfo) => {
    await session.openEditor();
    const { page } = session;
    const engine = await page.evaluate(() => !!(window as any).PM.EngineRender);
    await useViewer(page, session.userData, variant);
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
        /* The composition on screen: the engine's presentation (at the stage
           for the current viewer, over a fork's own canvas inside
           stage-inner), or today's WebGL canvas with the engine off. */
        const native = engine ? stage.querySelector<HTMLCanvasElement>('#engine-present') : inner.querySelector<HTMLCanvasElement>(':scope > #gl');
        if (!native || getComputedStyle(native).visibility !== 'visible') return { hash, visible, picture: false };
        const copy = document.createElement('canvas'); copy.width = native.width; copy.height = native.height;
        const ctx = copy.getContext('2d')!;
        ctx.drawImage(native, 0, 0);
        const css = native.getBoundingClientRect(), innerBox = inner.getBoundingClientRect();
        const pixel = (x: number, y: number) => {
          const sx = Math.floor((innerBox.left + x * zoom - css.left) * copy.width / css.width);
          const sy = Math.floor((innerBox.top + y * zoom - css.top) * copy.height / css.height);
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

    /* The live viewer's geometry, overlay, and composition bitmap from its
       DOM. The picture canvas is the engine's presentation with the engine
       on (at the stage for the current viewer, over a fork's own canvas
       inside stage-inner), and today's WebGL canvas with it off. */
    const live = () => page.evaluate(({ engine, layer, zoom }) => {
      const PM = (window as any).PM;
      const panel = PM.panelInst.viewer.el as HTMLElement;
      const stage = panel.querySelector<HTMLElement>('#stage')!;
      const inner = panel.querySelector<HTMLElement>('#stage-inner')!;
      const overlay = panel.querySelector<HTMLCanvasElement>('#overlay')!;
      const hashCanvas = (canvas: HTMLCanvasElement) => {
        const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
        let hash = 2166136261;
        for (let index = 0; index < data.length; index++) hash = Math.imul(hash ^ data[index]!, 16777619) >>> 0;
        return hash;
      };
      const stageBox = stage.getBoundingClientRect(), innerBox = inner.getBoundingClientRect();
      /* The selection outline on the overlay as drawn now: the first painted
         pixel within 3 CSS px of the layer's left edge, on a row clear of the
         handles (stage CSS px; null when none is drawn there). */
      const overlayBox = overlay.getBoundingClientRect();
      const density = overlay.width / overlayBox.width;
      const edge = innerBox.left - overlayBox.left + layer.left * zoom;
      const row = innerBox.top - overlayBox.top + (layer.top + 20) * zoom;
      const rowData = overlay.getContext('2d')!.getImageData(0, Math.round(row * density), overlay.width, 1).data;
      let outline: number | null = null;
      for (let x = Math.floor((edge - 3) * density); x <= Math.ceil((edge + 3) * density); x++) {
        if (x >= 0 && x < overlay.width && rowData[x * 4 + 3]! > 0) { outline = x / density; break; }
      }
      const native = engine ? stage.querySelector<HTMLCanvasElement>('#engine-present') : inner.querySelector<HTMLCanvasElement>(':scope > #gl');
      let present = null;
      if (native && getComputedStyle(native).visibility === 'visible') {
        const css = getComputedStyle(native), box = native.getBoundingClientRect();
        const todays = native.dataset.todaysCanvas?.split(' ').map(Number);
        if (native.parentElement === stage && todays?.length === 6) {
          /* A stage-level presentation shows the composition where today's
             canvas sits (the viewer tags it per frame); the card shows that
             canvas, at today's box and backing size. */
          const [left, top, width, height, renderWidth, renderHeight] = todays as [number, number, number, number, number, number];
          const sx = native.width / box.width, sy = native.height / box.height;
          const bitmap = document.createElement('canvas'); bitmap.width = Math.round(renderWidth); bitmap.height = Math.round(renderHeight);
          bitmap.getContext('2d')!.drawImage(native, left * sx, top * sy, width * sx, height * sy, 0, 0, bitmap.width, bitmap.height);
          present = {
            width: bitmap.width, height: bitmap.height, css: [`${width}px`, `${height}px`], hash: hashCanvas(bitmap),
            parent: native.parentElement?.id,
            offset: [box.left - stageBox.left + left, box.top - stageBox.top + top],
          };
        } else {
          const bitmap = document.createElement('canvas'); bitmap.width = native.width; bitmap.height = native.height;
          bitmap.getContext('2d')!.drawImage(native, 0, 0);
          present = {
            width: native.width, height: native.height, css: [css.width, css.height], hash: hashCanvas(bitmap),
            parent: native.parentElement?.id,
            // Where it sits in the stage, in stage CSS px.
            offset: [box.left - stageBox.left, box.top - stageBox.top],
          };
        }
      }
      return {
        inner: [inner.style.left, inner.style.top, inner.style.width, inner.style.height],
        overlay: { width: overlay.width, height: overlay.height, css: [overlay.style.width, overlay.style.height], parent: overlay.parentElement?.id, hash: hashCanvas(overlay) },
        outline: outline === null ? null : Math.round((outline - edge) * density),
        present,
        zoom: PM.Kernel.services.get('viewer').shown,
      };
    }, { engine, layer: LAYER, zoom: ZOOM });
    if (engine) await expect(page.locator('#panel-viewer #engine-present')).toHaveCSS('visibility', 'visible');
    if (engine && variant.ref) {
      // A fork's presentation runs in the presentation worker, as the
      // built-in viewer's does: its page canvas is a transferred placeholder.
      expect(await page.evaluate(() => {
        const native = document.querySelector<HTMLCanvasElement>('#panel-viewer #engine-present')!;
        try { native.getContext('2d'); return 'page'; } catch { return 'worker'; }
      })).toBe('worker');
    }
    const liveArtifact = testInfo.outputPath('viewer-live.png');
    await page.locator('#panel-viewer #stage').screenshot({ path: liveArtifact });
    await testInfo.attach('Live viewer', { path: liveArtifact, contentType: 'image/png' });
    /* Hashes compare two fresh drawings of the overlay. Chromium can move a
       2D canvas that is read back (the samples above) to another rasterizer,
       and a presented engine frame redraws the overlay, so an old drawing and
       a new one of the same state can differ in edge coverage alone. The
       drawing kept while the Library opens is checked by its outline
       position before the redraw. */
    const redrawOverlay = () => page.evaluate(() => new Promise<void>(resolve => {
      (window as any).PM.bus.emit('overlay');
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    }));
    // The overlay's own pixels, kept beside any difference the hashes report.
    const overlayPng = () => page.evaluate(() => ((window as any).PM.panelInst.viewer.el as HTMLElement).querySelector<HTMLCanvasElement>('#overlay')!.toDataURL());
    const attachOverlay = async (name: string) => testInfo.attach(name, { body: Buffer.from((await overlayPng()).split(',')[1]!, 'base64'), contentType: 'image/png' });
    await redrawOverlay();
    const before = await live();
    await attachOverlay('Live overlay before the Library');
    expect(before.zoom).toBe(ZOOM);
    expect(before.inner[2]).toBe(`${COMP.w * ZOOM}px`);
    expect(before.inner[3]).toBe(`${COMP.h * ZOOM}px`);
    expect(before.present).not.toBeNull();
    expect(before.outline).not.toBeNull();

    await page.evaluate(() => (window as any).PM.LibraryUI.open());
    const card = page.locator('.library-card[data-panel-id="viewer"]');
    await card.scrollIntoViewIfNeeded();
    await expect(card.locator('.library-clone')).toBeAttached();
    await expect(card.locator('[data-library-engine-frame]')).toHaveCount(0);

    // Compare the same non-hover state after the Library has loaded its fonts.
    await clearViewerHover(page);
    await page.evaluate(() => document.fonts.ready);
    await expect.poll(stableViewer, { timeout: 10_000 }).toBe(true);
    // The live viewer kept its overlay while the Library opened: the drawing
    // it holds now has the outline where the fresh one before had it.
    const kept = await live();
    await attachOverlay('Live overlay kept with the Library open');
    expect(kept.outline).not.toBeNull();
    expect(Math.abs(kept.outline! - before.outline!)).toBeLessThanOrEqual(1);
    expect({ ...kept, overlay: { ...kept.overlay, hash: 0 }, outline: 0 }).toEqual({ ...before, overlay: { ...before.overlay, hash: 0 }, outline: 0 });
    // And its zoom, pan, picture and overlay are today's, pixel for pixel.
    await redrawOverlay();
    const after = await live();
    await attachOverlay('Live overlay with the Library open');
    expect(after).toEqual(before);

    const clone = await card.evaluate((cardElement, { engine, layer, zoom, comp }) => {
      const root = cardElement.querySelector<HTMLElement>('.library-clone')!;
      const stage = root.querySelector<HTMLElement>('[data-library-source-id="stage"]')!;
      const inner = root.querySelector<HTMLElement>('[data-library-source-id="stage-inner"]')!;
      const overlay = root.querySelector<HTMLCanvasElement>('[data-library-source-id="overlay"]')!;
      const present = root.querySelector<HTMLCanvasElement>(engine ? 'canvas[data-library-engine-snapshot]' : 'canvas[data-library-source-id="gl"]');
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
      const innerBox = inner.getBoundingClientRect();
      /* A composition point on a copied canvas, through the card's boxes
         (the card scales the whole clone). */
      const pixel = (canvas: HTMLCanvasElement, compX: number, compY: number) => {
        const r = canvas.getBoundingClientRect();
        const x = Math.floor((innerBox.left + compX / comp.w * innerBox.width - r.left) * canvas.width / r.width);
        const y = Math.floor((innerBox.top + compY / comp.h * innerBox.height - r.top) * canvas.height / r.height);
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
          css: [getComputedStyle(present).width, getComputedStyle(present).height],
          hash: hashCanvas(present),
          parent: present.parentElement?.dataset.librarySourceId,
          visibility: getComputedStyle(present).visibility,
          inside: pixel(present, layer.left + 3, layer.top + 20),
          outside: pixel(present, layer.left - 3, layer.top + 20),
          center: pixel(present, (layer.left + layer.right) / 2, (layer.top + layer.bottom) / 2),
        },
        edge, outline,
      };
    }, { engine, layer: LAYER, zoom: ZOOM, comp: COMP });

    // No refit: the clone's stage-inner is the live one, and the copied overlay is the live overlay, pixel for pixel.
    expect(clone.inner).toEqual(after.inner);
    expect(clone.overlay).toEqual({ ...after.overlay, parent: 'stage' });
    expect(after.overlay.parent).toBe('stage');
    // The selection outline sits on the layer's left edge in stage coordinates.
    expect(clone.outline).not.toBeNull();
    expect(Math.abs(clone.outline! - clone.edge)).toBeLessThanOrEqual(3);
    // The composition is in the card, with the engine on or off and for
    // every viewer: the live picture, pixel for pixel, where the live viewer
    // shows it.
    const present = clone.present!;
    expect(present).not.toBeNull();
    expect(present.visibility).toBe('visible');
    const shown = after.present!;
    expect({ width: present.width, height: present.height, css: present.css, hash: present.hash, parent: present.parent })
      .toEqual({ width: shown.width, height: shown.height, css: shown.css, hash: shown.hash, parent: 'stage-inner' });
    // The picture keeps its place in the source stage's CSS box (today's
    // composition canvas box), even though the Library card itself has a
    // different fixed size.
    const cardScale = clone.innerBox.width / (COMP.w * ZOOM);
    expect(clone.presentBox!.left - clone.stage.left).toBeCloseTo(shown.offset[0]! * cardScale, 1);
    expect(clone.presentBox!.top - clone.stage.top).toBeCloseTo(shown.offset[1]! * cardScale, 1);
    expect(clone.presentBox!.width).toBeCloseTo(parseFloat(shown.css[0]!) * cardScale, 1);
    expect(clone.presentBox!.height).toBeCloseTo(parseFloat(shown.css[1]!) * cardScale, 1);
    // Picture and selected outline agree at the live zoom and pan.
    expect(present.inside[2]).toBeGreaterThan(180);
    expect(present.inside[0]).toBeLessThan(80);
    expect(present.outside[0]! + present.outside[1]! + present.outside[2]!).toBeLessThan(60);
    expect(present.center[2]).toBeGreaterThan(180);
    // The layer is visible in the card, not cropped away.
    const point = { x: clone.innerBox.left + (LAYER.left + LAYER.right) / 2 * ZOOM * cardScale, y: clone.innerBox.top + (LAYER.top + LAYER.bottom) / 2 * ZOOM * cardScale };
    for (const bounds of [clone.stage, clone.frameBox] as Box[]) {
      expect(point.x).toBeGreaterThan(bounds.left);
      expect(point.x).toBeLessThan(bounds.left + bounds.width);
      expect(point.y).toBeGreaterThan(bounds.top);
      expect(point.y).toBeLessThan(bounds.top + bounds.height);
    }
    const artifact = testInfo.outputPath('library.png');
    await card.screenshot({ path: artifact });
    await testInfo.attach('Library viewer card', { path: artifact, contentType: 'image/png' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
