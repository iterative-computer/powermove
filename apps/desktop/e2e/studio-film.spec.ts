import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { copyFileSync, statSync } from 'node:fs';
import { expect, repoRoot, test } from './helpers/app';

/* The acceptance film (docs/3d-studio-redesign.md, 2a), made entirely in
   Powermove and exported through Studio's Final tier: an original tablet
   ("Slate": a rounded slab with a live screen), an animated screen
   composition, the studio set, auto-studio softboxes plus one placed by
   pointing, a turntable then a push-in, and 2D titles over the 3D. */
test('a product film is made and exported entirely in Powermove', async ({ session }, info) => {
  test.setTimeout(600_000);
  await session.openEditor();
  const { page } = session;
  const output = path.join(session.userData, 'slate-film.mp4');
  await session.app.evaluate(({ dialog }, file) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: file }); }, output);

  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-film');
    const ok = (r: any) => { if (!r.ok) throw new Error(r.message); return r; };
    PM.proj.w = 1280; PM.proj.h = 720; PM.proj.fps = 30; PM.proj.dur = 6; PM.proj.bg = '#F6F5F8';

    // The screen: soft colour fields drifting across a dark display.
    const screen = PM.mkProject({ name: 'Slate screen', w: 1024, h: 1400, fps: 30, dur: 6, bg: '#07070D' });
    const fields: [string, number, number, number][] = [['#4B5BFF', 200, 300, 1], ['#9A4BFF', 760, 900, 1.6], ['#2BB3FF', 420, 1200, 2.2]];
    screen.layers = fields.map(([color, x, y, phase]) => {
      const l = PM.mkLayer('solid', { from: 0, dur: 6, d: { color, w: 520, h: 520 } }, screen);
      PM.animate(l, 'position.x', [{ t: 0, v: x }, { t: 6, v: 1024 - x }]);
      PM.animate(l, 'position.y', [{ t: 0, v: y }, { t: 6, v: y + 260 * Math.sin(phase) }]);
      PM.animate(l, 'opacity', [{ t: 0, v: 55 }, { t: 3, v: 85 }, { t: 6, v: 55 }]);
      return l;
    });
    PM.proj.comps = { ...(PM.proj.comps || {}), slate_screen: { ...screen, id: 'slate_screen' } };

    // The product: a rounded aluminium slab and its display, grouped.
    ok(api.scene3d.edit({ operation: 'add_object', object: { id: 'slate_body', name: 'Slate body', source: { primitive: 'roundedBox', parameters: { width: 1.35, height: 0.07, depth: 1.95, radius: 0.03 } } } }));
    ok(api.scene3d.edit({ operation: 'apply_material', target: 'slate_body', preset: 'anodized' }));
    ok(api.scene3d.edit({ operation: 'add_object', object: { id: 'slate_screen_glass', name: 'Slate display', source: { primitive: 'plane', parameters: { width: 1.22, height: 1.82 } }, p: { y: 0.036, rx: -90 }, material: { screen: 'slate_screen', p: { color: '#050507', roughness: 0.6, clearcoat: 1, clearcoatRoughness: 0.012, specular: 0, emissiveIntensity: 1.3 } } } }));
    ok(api.scene3d.edit({ operation: 'drop_to_floor', target: 'slate_body' }));
    ok(api.scene3d.edit({ operation: 'update_object', target: 'slate_screen_glass', patch: { p: { y: 0.0715 } } }));
    PM.Edit.apply({ type: 'group_layers', targets: ['slate_body', 'slate_screen_glass'], name: 'Slate' });
    const group = PM.proj.layers.find((l: any) => l.type === 'group');

    // The studio, one placed highlight, quality, and the moves.
    ok(api.scene3d.edit({ operation: 'auto_studio' }));
    ok(api.scene3d.edit({ operation: 'light_highlight', at: [0.3, 0.07, -0.4], normal: [0, 1, 0], intensity: 4 }));
    ok(api.scene3d.edit({ operation: 'set_environment', patch: { samples: 16 } }));
    group.p['rotation.y'].v = 90;
    ok(api.scene3d.edit({ operation: 'shot', shot: 'turntable', target: group.id, start: 0, duration: 3.2, degrees: 270 }));
    ok(api.scene3d.edit({ operation: 'shot', shot: 'push_in', start: 3.2, duration: 2.8, amount: 0.28 }));

    // Titles over the 3D.
    const title = PM.mkLayer('text', { from: 3.4, dur: 2.6, p: { 'position.x': 640, 'position.y': 120 }, d: { text: 'Slate', size: 64, color: '#16161A', align: 'center' } });
    PM.animate(title, 'opacity', [{ t: 0, v: 0 }, { t: 0.7, v: 100 }]); // layer time
    const line = PM.mkLayer('text', { from: 3.8, dur: 2.2, p: { 'position.x': 640, 'position.y': 180 }, d: { text: 'Rendered in Powermove Studio', size: 24, color: '#6A63E8', align: 'center' } });
    PM.animate(line, 'opacity', [{ t: 0, v: 0 }, { t: 0.7, v: 100 }]);
    PM.addLayer(line, 0); PM.addLayer(title, 0);
    PM.touch(); PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);

  const started = Date.now();
  const result = await page.evaluate(() => (window as any).PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 30, range: 'all', quality: 'high', audio: false, mblur: false }));
  const seconds = (Date.now() - started) / 1000;
  expect(result).toEqual({ cancelled: false });
  expect(statSync(output).size).toBeGreaterThan(100_000);
  const titled = await page.evaluate(() => (window as any).PM.proj.layers.filter((l: any) => l.type === 'text').length);
  expect(titled).toBe(2);
  const ffprobe = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
  const probe = execFileSync(ffprobe, ['-hide_banner', '-i', output, '-f', 'null', '-'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).toString();
  void probe;
  copyFileSync(output, info.outputPath('slate-film.mp4'));
  await info.attach('export-seconds', { body: String(seconds), contentType: 'text/plain' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
