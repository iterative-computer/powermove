import { mkdir } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, repoRoot, test } from './helpers/app';

/* Extruded text and shapes, through the interface: the Extrude switch (which
   makes a 2D layer 3D), Geometry Options with the Bevel Style profiles, live
   depth while dragging, Front/Bevel/Side/Back Material Options with the shared
   source picker, "Use Front for All", and the Timeline's Geometry Options and
   Materials groups. Set PM_SHOT_DIR to also write the review screenshots. */

const shots = process.env.PM_SHOT_DIR;
async function shot(page: any, name: string, clip?: { x: number; y: number; width: number; height: number }) {
  if (!shots) return;
  await mkdir(shots, { recursive: true });
  await page.waitForTimeout(250);
  await page.screenshot({ path: `${shots}/${name}.png`, clip });
}

async function setup(session: any, threeD = false) {
  await session.openEditor();
  const page = session.page;
  await page.evaluate((threeD: boolean) => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.();
    const api = PM.Kernel.api('extrusion-ui-e2e').scene3d;
    PM.Edit.apply({ type: 'add_layer', id: 'title', layerType: 'text', name: 'Powermove', content: { text: 'Powermove', font: 'Arial', weight: 700, size: 220, align: 'center', color: '#DAD7FF' }, properties: { 'position.x': 960, 'position.y': 470 } });
    PM.Edit.apply({ type: 'add_layer', id: 'star', layerType: 'shape', name: 'Star', content: { shape: 'star', w: 240, h: 240, points: 5, radius: 20, color: '#F2C94C' }, properties: { 'position.x': 960, 'position.y': 800 } });
    if (threeD) PM.Edit.apply([{ type: 'set_layer', target: 'title', patch: { threeD: true } }, { type: 'set_layer', target: 'star', patch: { threeD: true } }]);
    for (const args of [{ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 8 } } }, { operation: 'add_light', light: { type: 'point', p: { x: -3, y: 3, z: 4, intensity: 80, decay: 2 } } }, { operation: 'add_light', light: { type: 'spot', p: { x: 4, y: 4, z: 4, intensity: 120, targetX: 0, targetY: 0, targetZ: 0, angle: 50, color: '#9CAFFF' } } }]) {
      const result = api.edit(args); if (!result.ok) throw new Error(result.message);
    }
    PM.selectLayers(['title']);
    try { localStorage.setItem('pm.tip.propertyKeys', '1'); } catch {}
  }, threeD);
  await page.waitForTimeout(500);
}

const extrusion = (page: any, id = 'title') => page.evaluate((id: string) => {
  const layer = (window as any).PM.L(id);
  return { threeD: !!layer.threeD, data: layer.d.extrusion ? JSON.parse(JSON.stringify(layer.d.extrusion)) : null };
}, id);
/** Show the Geometry Options section at the top of Properties. */
const reveal = (page: any) => page.locator('[data-extrusion-section]').evaluate((el: Element) => el.scrollIntoView({ block: 'start' }));
const tilt = (page: any, id = 'title') => page.evaluate((id: string) => {
  (window as any).PM.Edit.apply([{ type: 'set_property', target: id, path: 'rotation.y', value: -32 }, { type: 'set_property', target: id, path: 'rotation.x', value: 14 }]);
}, id);

test('Extrude makes a 2D text layer 3D and extruded; Geometry Options edit depth, bevel and quality', async ({ session }) => {
  test.setTimeout(120_000);
  const page = session.page;
  await setup(session);
  const section = page.locator('[data-extrusion-section]');
  await expect(section).toBeVisible();
  await expect(section.locator('.sec')).toHaveText('Geometry Options');
  // A 2D layer: the switch explains that it will also make the layer 3D.
  const toggle = section.locator('[data-extrude-toggle]');
  await expect(toggle).toHaveAttribute('title', /makes the layer 3D too/);
  await expect(page.locator('[data-surface-materials]')).toHaveCount(0);
  await reveal(page);
  await toggle.hover();
  await page.waitForTimeout(900);
  await shot(page, '02-after-01-extrude-affordance-2d-light');

  // One click: 3D and extruded, in one undo step.
  await toggle.getByRole('radio', { name: 'On' }).click();
  await expect.poll(async () => (await extrusion(page)).data?.enabled).toBe(true);
  expect((await extrusion(page)).threeD).toBe(true);
  expect((await extrusion(page)).data.p.depth.v).toBe(40);
  await expect(section.locator('[data-scene-channel="geo.depth"]')).toBeVisible();
  await expect(page.locator('[data-surface-materials] [data-surface]')).toHaveCount(4);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => extrusion(page)).toEqual({ threeD: false, data: null });
  await page.evaluate(() => (window as any).PM.hist.redo());
  await expect.poll(async () => (await extrusion(page)).threeD).toBe(true);
  await tilt(page);
  await reveal(page);
  await page.mouse.move(700, 300);
  await page.waitForTimeout(400);
  await shot(page, '02-after-02-extruded-light');

  // Dragging Extrusion Depth updates the model live, before the pointer is released.
  const depth = section.locator('[data-scene-channel="geo.depth"] .num').first();
  const box = (await depth.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  for (let step = 1; step <= 8; step++) await page.mouse.move(box.x + box.width / 2 + step * 10, box.y + box.height / 2);
  await expect.poll(async () => (await extrusion(page)).data.p.depth.v).toBeGreaterThan(60);
  await shot(page, '02-after-03-depth-drag-midway-light');
  await page.mouse.up();
  const dragged = (await extrusion(page)).data.p.depth.v;
  expect(dragged).toBeGreaterThan(60);
  // One drag is one undo step.
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => (await extrusion(page)).data.p.depth.v).toBe(40);
  await page.evaluate(() => (window as any).PM.hist.redo());

  // Bevel Style: four profiles; Bevel Depth reads as inactive with None.
  const styles = section.locator('.bevel-styles button');
  await expect(styles).toHaveCount(4);
  await expect(styles.nth(0)).toHaveAttribute('aria-checked', 'true');
  await expect(section.locator('[data-scene-channel="geo.bevelDepth"]').locator('..')).toHaveClass(/inactive/);
  await page.evaluate(() => (window as any).PM.Edit.apply({ type: 'set_property', target: 'title', path: 'geo.bevelDepth', value: 6 }));
  for (const [index, style] of ['angular', 'concave', 'convex', 'none'].entries()) {
    await section.locator(`[data-bevel-style="${style}"]`).click();
    await expect.poll(async () => (await extrusion(page)).data.p.bevelStyle.v).toBe(style);
    await expect(section.locator(`[data-bevel-style="${style}"]`)).toHaveAttribute('aria-checked', 'true');
    await shot(page, `02-after-04-bevel-${index + 1}-${style}-light`);
  }
  // Arrow keys move between styles.
  await section.locator('[data-bevel-style="none"]').focus();
  await page.keyboard.press('ArrowRight');
  await expect.poll(async () => (await extrusion(page)).data.p.bevelStyle.v).toBe('angular');

  // Quality.
  await section.locator('[data-geometry-property="geo.quality"] .pm-select, [data-geometry-property="geo.quality"] select').first().click();
  await page.locator('.pm-menu [role="option"], .pm-menu [role="menuitemradio"], .pm-menu button').filter({ hasText: 'High' }).first().click();
  await expect.poll(async () => (await extrusion(page)).data.p.quality.v).toBe('high');

  // Turning Extrude off keeps the authored settings for turning it back on.
  await toggle.getByRole('radio', { name: 'Off' }).click();
  await expect.poll(async () => (await extrusion(page)).data.enabled).toBe(false);
  await expect(page.locator('[data-surface-materials]')).toHaveCount(0);
  await toggle.getByRole('radio', { name: 'On' }).click();
  await expect.poll(async () => (await extrusion(page)).data.p.quality.v).toBe('high');
  expect((await extrusion(page)).data.p.depth.v).toBe(dragged);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Material Options paint Front, Bevel, Side and Back; Use Front for All copies Front; Timeline groups', async ({ session }) => {
  test.setTimeout(150_000);
  const page = session.page;
  await setup(session, true);
  // A hidden video to use on Front.
  const file = path.join(session.userData, 'front.webm');
  const frames = Buffer.concat(Array.from({ length: 20 }, (_, i) => {
    const frame = Buffer.alloc(64 * 64 * 3);
    for (let p = 0; p < frame.length; p += 3) { const x = (p / 3) % 64; frame[p] = x < 32 ? 255 : 40; frame[p + 1] = i < 10 ? 80 : 200; frame[p + 2] = x < 32 ? 60 : 255; }
    return frame;
  }));
  execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', '10', '-i', 'pipe:0', '-c:v', 'libvpx-vp9', '-lossless', '1', '-g', '1', file], { input: frames });
  await page.evaluate(async (bytes: number[]) => {
    const PM = (window as any).PM;
    const asset = await PM.assets.add(new File([new Uint8Array(bytes)], 'Sunset Clip.webm', { type: 'video/webm' }), { silent: true });
    const add = PM.Edit.apply({ type: 'add_layer', id: 'movie', layerType: 'video', name: 'Sunset Clip', visible: false, content: { asset: asset.id, w: 64, h: 64, fit: 'stretch' } });
    if (!add.ok) throw new Error(add.message);
    PM.selectLayers(['title']);
  }, [...(await import('node:fs/promises').then(fs => fs.readFile(file)))]);
  const section = page.locator('[data-extrusion-section]');
  await expect(section.locator('[data-extrude-toggle]')).toHaveAttribute('title', /^Extrude — Gives the layer real depth/);
  await section.locator('[data-extrude-toggle]').getByRole('radio', { name: 'On' }).click();
  await expect.poll(async () => (await extrusion(page)).data?.enabled).toBe(true);
  await page.evaluate(() => (window as any).PM.Edit.apply([{ type: 'set_property', target: 'title', path: 'geo.depth', value: 70 }, { type: 'set_property', target: 'title', path: 'geo.bevelStyle', value: 'convex' }, { type: 'set_property', target: 'title', path: 'geo.bevelDepth', value: 5 }]));
  await tilt(page);

  const surfaces = page.locator('[data-surface-materials]');
  await expect(surfaces.locator('.mat-name')).toHaveText(['Front', 'Bevel', 'Side', 'Back']);
  await expect(page.locator('[data-use-front]')).toBeDisabled();
  // Front is open with the shared source picker.
  const front = surfaces.locator('[data-surface="front"]');
  await front.locator('[data-material-slot="0:color"]').scrollIntoViewIfNeeded();
  await front.locator('[data-material-slot="0:color"] button').click();
  const picker = page.locator('[data-material-picker="color"]');
  await expect(picker).toBeVisible();
  await expect(picker.locator('[data-option="layer:movie"]')).toBeVisible();
  await expect(picker.locator('[data-option="layer:title"]')).toHaveCount(0);
  await page.waitForTimeout(800);
  await shot(page, '02-after-05-front-source-picker-light');
  await picker.locator('[data-option="layer:movie"]').click();
  await expect.poll(async () => (await extrusion(page)).data.surfaces.front.maps.color).toEqual({ layerId: 'movie' });
  await expect(front.locator('[data-material-slot="0:color"] button')).toHaveAttribute('aria-label', /Sunset Clip/);
  // The source shows its picture, and a Timeline layer drag can target any surface slot.
  await expect(front.locator('[data-material-slot="0:color"] [data-material-thumb]')).toHaveAttribute('data-material-thumb', 'ready', { timeout: 15_000 });
  const drop = await page.evaluate(() => (window as any).PM.Kernel.services.get('materials').canAssignLayer({ layerId: 'title', materialIndex: 2, slot: 'color' }, 'movie'));
  expect(drop).toEqual({ ok: true });
  await expect(front.locator('[data-material-slot="0:color"]')).toHaveAttribute('data-material-slot-index', '0');

  // Side: its own color and finish; the first edit installs the property.
  await surfaces.locator('[data-surface="side"] .mat-twirl').click();
  await page.evaluate(() => {
    const api = (window as any).PM.Kernel.api('extrusion-ui-e2e').scene3d;
    const result = api.setSurfaceMaterialProperties('title', 'side', { color: '#E44828', roughness: 0.3, metallic: 0.4 });
    if (!result.ok) throw new Error(result.message);
  });
  const roughness = surfaces.locator('[data-surface="side"] [data-surface-property="ms.side.roughness"]');
  await expect(roughness).toBeVisible();
  // Keyframe Bevel Opacity from its stopwatch (it installs at the inherited value).
  await surfaces.locator('[data-surface="bevel"] .mat-twirl').click();
  await surfaces.locator('[data-surface="bevel"] [data-surface-property="ms.bevel.metallic"] .stopwatch').click();
  await expect.poll(async () => (await extrusion(page)).data.surfaces.bevel?.properties?.metallic?.kf?.length).toBe(1);
  await surfaces.locator('[data-surface="side"]').scrollIntoViewIfNeeded();
  await page.mouse.move(700, 300);
  await page.waitForTimeout(800);
  await shot(page, '02-after-06-surface-materials-video-front-light');
  await expect(page.locator('[data-use-front]')).toBeEnabled();

  // Use Front for All: every surface takes Front's source; one undo step.
  await page.locator('[data-use-front]').click();
  await expect.poll(async () => (await extrusion(page)).data.surfaces.side.maps.color).toEqual({ layerId: 'movie' });
  const copied = (await extrusion(page)).data.surfaces;
  expect(copied.back.maps.color).toEqual({ layerId: 'movie' });
  expect(copied.side.properties.color).toBeUndefined();
  expect(copied.bevel.properties.metallic).toBeUndefined();
  await expect(page.locator('[data-use-front]')).toBeDisabled();
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => (await extrusion(page)).data.surfaces.side.properties?.color?.v).toBe('#E44828');
  expect((await extrusion(page)).data.surfaces.side.maps?.color).toBeUndefined();

  // Reset puts Side back to the layer's own look.
  await surfaces.locator('[data-surface="side"] .mat-reset').click();
  await expect.poll(async () => JSON.stringify((await extrusion(page)).data.surfaces.side)).toBe(JSON.stringify({ maps: {}, properties: {} }));

  // Keyframed depth, then the Timeline's Geometry Options and Materials groups.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const result = PM.Edit.apply({ type: 'replace_keyframes', target: 'title', path: 'geo.depth', keyframes: [{ time: 0, value: 10 }, { time: 1.5, value: 120 }] });
    if (!result.ok) throw new Error(result.message);
    PM.selectLayers(['title']);
  });
  const cvBox = () => page.evaluate(() => { const r = (window as any).PM.Kernel.services.get('timeline').cv.getBoundingClientRect(); return { x: r.left, y: r.top }; });
  const rows = () => page.evaluate(() => {
    const T = (window as any).PM.Kernel.services.get('timeline');
    return { rows: T.rows.map((r: any) => ({ kind: r.kind, key: r.key, label: r.label, layer: r.L?.id, virtual: !!r.virtual })), ruler: T.ruler, row: T.row, scrollY: T.scrollY, gut: T.gut, valueX: T.propertyValueX };
  });
  const point = async (match: (r: any) => boolean, x: (s: any) => number) => {
    // Scroll the layer list until the row is in view, as a person would.
    for (let attempt = 0; attempt < 12; attempt++) {
      const state = await rows(); const index = state.rows.findIndex(match);
      expect(index).toBeGreaterThanOrEqual(0);
      const box = await cvBox(), height = await page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').cv.getBoundingClientRect().height);
      const y = state.ruler + index * state.row - state.scrollY;
      if (y >= state.ruler && y + state.row <= height) return { x: box.x + x(state), y: box.y + y + state.row / 2 };
      await page.mouse.move(box.x + 200, box.y + state.ruler + 40);
      await page.mouse.wheel(0, y + state.row > height ? state.row * 3 : -state.row * 3);
      await page.waitForTimeout(120);
    }
    throw new Error('Row never scrolled into view');
  };
  const collapsed = await page.evaluate(() => (window as any).PM.Kernel.api('extrusion-ui-e2e').uiState.getLayerCollapsed((window as any).PM.L('title')));
  if (collapsed) {
    const twirl = await point(r => r.kind === 'layer' && r.layer === 'title', () => 87);
    await page.mouse.click(twirl.x, twirl.y);
  }
  const titleRows = async () => (await rows()).rows.filter((r: any) => r.layer === 'title' && r.kind !== 'layer').map((r: any) => `${r.kind}:${r.label}`);
  await expect.poll(titleRows).toEqual(expect.arrayContaining(['heading:Geometry Options', 'prop:Extrusion Depth', 'prop:Bevel Style', 'prop:Bevel Depth', 'prop:Hole Bevel Depth', 'prop:Geometry Quality', 'heading:Materials', 'material:Front', 'material:Bevel', 'material:Side', 'material:Back']));
  // Geometry rows appear once, inside their group.
  expect((await titleRows()).filter(row => row === 'prop:Extrusion Depth')).toHaveLength(1);
  // Open Side: Emissive, Color and the six options as stand-ins.
  const side = await point(r => r.kind === 'material' && r.layer === 'title' && r.key === 'material:2:color', () => 148);
  await page.mouse.click(side.x, side.y);
  await expect.poll(async () => (await rows()).rows.filter((r: any) => r.layer === 'title' && r.key?.startsWith('ms.side.')).length).toBe(7);
  // A stand-in diamond installs and keys the surface property.
  const sideRough = await point(r => r.layer === 'title' && r.key === 'ms.side.roughness', () => 148.5);
  await page.mouse.click(sideRough.x, sideRough.y);
  await expect.poll(async () => (await extrusion(page)).data.surfaces.side?.properties?.roughness?.kf?.length ?? 0).toBe(1);
  // The Geometry Options group with its keyframed depth, mid-animation.
  await page.evaluate(() => (window as any).PM.setTime(0.9, { force: true }));
  await point(r => r.layer === 'title' && r.key === 'section:geometry', () => 200);
  const geometryTop = await page.evaluate(() => {
    const T = (window as any).PM.Kernel.services.get('timeline');
    return T.rows.findIndex((r: any) => r.L?.id === 'title' && r.key === 'section:geometry') * T.row;
  });
  for (let attempt = 0; attempt < 8; attempt++) {
    const { scrollY } = await rows();
    if (scrollY <= geometryTop - 40) break;
    const box = await cvBox();
    await page.mouse.move(box.x + 200, box.y + 120);
    await page.mouse.wheel(0, -60);
    await page.waitForTimeout(120);
  }
  await page.mouse.move(900, 300);
  await shot(page, '02-after-07-timeline-keyframed-depth-light');
  // The Front pop-up opens the same picker, with the clip chosen.
  const popup = await point(r => r.layer === 'title' && r.key === 'material:0:color', s => s.valueX + 30);
  await page.mouse.click(popup.x, popup.y);
  await expect(page.locator('[data-material-picker="color"] [data-option="layer:movie"]')).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('Escape');

  // The video layer wears the "used as material" badge naming the text layer.
  const usage = await page.evaluate(() => (window as any).PM.Kernel.services.get('materials')?.usage('movie'));
  expect(usage).toEqual([{ layerId: 'title', name: 'Powermove', uses: ['Front · Base Color'] }]);

  // Dark theme.
  await page.evaluate(() => { const PM = (window as any).PM; PM.theme.apply('dark'); PM.selectLayers(['title']); });
  await reveal(page);
  await shot(page, '02-after-08-geometry-options-dark');
  await surfaces.locator('[data-surface="front"]').scrollIntoViewIfNeeded();
  await shot(page, '02-after-09-surface-materials-dark');
  // The shape layer gets the same section.
  await page.evaluate(() => (window as any).PM.selectLayers(['star']));
  await expect(page.locator('[data-extrusion-section] [data-extrude-toggle]')).toBeVisible();
  await page.locator('[data-extrusion-section] [data-extrude-toggle]').getByRole('radio', { name: 'On' }).click();
  await expect.poll(async () => (await extrusion(page, 'star')).data?.enabled).toBe(true);
  await page.evaluate(() => (window as any).PM.Edit.apply([{ type: 'set_property', target: 'star', path: 'geo.bevelStyle', value: 'angular' }, { type: 'set_property', target: 'star', path: 'geo.bevelDepth', value: 6 }, { type: 'set_property', target: 'star', path: 'geo.depth', value: 60 }]));
  await tilt(page, 'star');
  await reveal(page);
  await shot(page, '02-after-10-shape-extruded-dark');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
