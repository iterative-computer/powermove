import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { test, expect, repoRoot } from './helpers/app';

const shots = '/tmp/pm-shots/studio3d/laneK';

test('bundled variable fonts and Arial extrude without losing the scene', async ({ session }) => {
  test.setTimeout(120_000);
  await mkdir(shots, { recursive: true });
  await session.openEditor();
  const page = session.page;
  const packaged = await Promise.all(['Inter', 'Geist', 'GeistMono'].map(async name => [name === 'GeistMono' ? 'Geist Mono' : name, [...await readFile(path.join(repoRoot, `assets/fonts/${name}-Variable.ttf`))]]));
  await page.evaluate(packaged => {
    const sources = new Map(packaged.map(([name, bytes]) => [name, new Uint8Array(bytes as number[]).buffer]));
    (window as any).PM.fontSource = (name: string) => sources.get(name);
  }, packaged);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({ name: 'Font regression', w: 640, h: 360, fps: 30, dur: 2 }));
    const api = PM.Kernel.api('font-regression').scene3d;
    PM.Edit.apply({ type: 'add_layer', id: 'title', layerType: 'text', content: { text: 'Studio Motion', font: 'Arial', weight: 700, size: 100, align: 'center', color: '#DAD7FF' }, properties: { 'position.x': 320, 'position.y': 180 } });
    api.setExtrusion('title', { enabled: true, depth: 40, bevelStyle: 'convex', bevelDepth: 3, quality: 'high' });
    api.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 8 } } });
    api.edit({ operation: 'add_light', light: { type: 'point', p: { x: -3, y: 3, z: 4, intensity: 80 } } });
  });
  for (const font of ['Arial', 'Inter', 'Geist', 'Geist Mono']) {
    const result = await page.evaluate(async font => {
      const PM = (window as any).PM;
      PM.Edit.apply({ type: 'set_content', target: 'title', patch: { font } });
      await PM.prepareFrame(0, { mblur: false });
      const canvas = PM.renderFrameTo(0, 640, 360, { mblur: false });
      const data = canvas.getContext('2d').getImageData(0, 0, 640, 360).data;
      let lit = 0;
      for (let i = 0; i < data.length; i += 4) if (Math.max(data[i], data[i + 1], data[i + 2]) > 40) lit++;
      return { png: canvas.toDataURL('image/png'), lit, status:PM.Studio.status() };
    }, font);
    await writeFile(`${shots}/font-${font.replaceAll(' ', '-')}.png`, Buffer.from(result.png.split(',')[1], 'base64'));
    expect(result.lit, `${font}: ${JSON.stringify(result.status)}`).toBeGreaterThan(600);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
