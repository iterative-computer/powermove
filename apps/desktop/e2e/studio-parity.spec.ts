import { mkdir, writeFile } from 'node:fs/promises';
import { expect, test } from './helpers/app';
import { partsGlb } from './helpers/parts-glb';
import { screenGlb } from '../src/renderer/src/core/scene3d/__fixtures__/screen-glb';

// The same scenes can be captured before and after replacing the renderer.
// Images stay outside the public repository.
const phase = process.env.POWERMOVE_PARITY_PHASE || 'studio';
const folder = '/tmp/pm-shots/studio3d/laneA';
test('main scene features survive the Studio renderer upgrade', async ({ session }) => {
  test.setTimeout(180_000);
  await mkdir(folder, { recursive: true });
  await session.openEditor();
  const page = session.page;
  const reset = () => page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({ name: '3D parity', w: 960, h: 540, fps: 30, dur: 3 }));
    const api = PM.Kernel.api('studio-parity').scene3d;
    const r = api.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 4 } } });
    if (!r.ok) throw new Error(r.message);
  });
  const capture = async (name: string, time = 0) => {
    const result = await page.evaluate(async (time: number) => {
      const PM = (window as any).PM; await PM.prepareFrame(time, { mblur: false });
      const canvas = PM.renderFrameTo(time, 960, 540, { mblur: false });
      const pixels = canvas.getContext('2d').getImageData(0, 0, 960, 540).data;
      let lit = 0;
      for (let i = 0; i < pixels.length; i += 4) if (Math.max(pixels[i], pixels[i + 1], pixels[i + 2]) > 35) lit++;
      return { png: canvas.toDataURL('image/png'), lit, studio: PM.Studio?.status() };
    }, time);
    await writeFile(`${folder}/${phase}-${name}.png`, Buffer.from(result.png.split(',')[1], 'base64'));
    expect(result.lit).toBeGreaterThan(500);
    if (phase !== 'main') {
      expect(result.studio?.ready).toBe(true);
      expect(result.studio?.frames).toBeGreaterThan(0);
    }
    return result.png;
  };
  await reset();
  await page.evaluate(async (bytes: number[]) => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-parity').scene3d;
    const r = await PM.cmd('3d.import-obj', new File([new Uint8Array(bytes)], 'Parts.glb', { type: 'model/gltf-binary' }));
    if (!r.ok) throw new Error(r.message);
    const model = PM.proj.layers.find((l: any) => l.d?.data?.object);
    const part = api.setPart(model.id, '0_2', { x: .25, ry: -12 });
    if (!part.ok) throw new Error(part.message);
  }, [...partsGlb()]);
  await capture('01-model-parts');

  await reset();
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-parity').scene3d;
    api.edit({ operation: 'set_camera', patch: { p: { z: 8 } } });
    const r = PM.Edit.apply({ type: 'add_layer', id: 'title', layerType: 'text', content: { text: 'Studio', font: 'Arial', weight: 700, size: 130, align: 'center', color: '#E7D7FF' }, properties: { 'position.x': 480, 'position.y': 280, 'rotation.y': -28, 'rotation.x': 12 } });
    if (!r.ok) throw new Error(r.message);
    api.setExtrusion('title', { enabled: true, depth: 80, bevelStyle: 'convex', bevelDepth: 5 });
    for (const [surface, color] of [['front', '#E7D7FF'], ['bevel', '#FFE088'], ['side', '#E44828'], ['back', '#3399FF']]) {
      const m = api.setSurfaceMaterialProperties('title', surface, { color, roughness: .35 });
      if (!m.ok) throw new Error(m.message);
    }
    api.edit({ operation: 'add_light', light: { type: 'point', p: { x: -3, y: 3, z: 4, intensity: 80 } } });
  });
  await capture('02-extruded-surfaces');

  await reset();
  await page.evaluate(async (bytes: number[]) => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-parity').scene3d;
    const r = await PM.cmd('3d.import-obj', new File([new Uint8Array(bytes)], 'Screen.glb', { type: 'model/gltf-binary' }));
    if (!r.ok) throw new Error(r.message);
    const model = PM.proj.layers.find((l: any) => l.d?.data?.object);
    PM.Edit.apply({ type: 'add_layer', id: 'paint', layerType: 'solid', visible: false, content: { w: 128, h: 128, color: '#22DD66' } });
    for (const slot of ['color', 'emissive']) {
      const m = api.setMaterialMap(model.id, 0, slot, { layerId: 'paint' });
      if (!m.ok) throw new Error(m.message);
    }
  }, [...screenGlb()]);
  await capture('03-layer-material');

  await reset();
  await page.evaluate(() => {
    const api = (window as any).PM.Kernel.api('studio-parity').scene3d;
    for (const args of [
      { operation: 'add_object', object: { id: 'hero', source: { primitive: 'sphere', parameters: { radius: .8 } }, material: { p: { color: '#C7C4FF', metalness: .4, roughness: .25 } } } },
      { operation: 'add_light', light: { type: 'point', p: { x: -2, y: 2, z: 3, intensity: 60, color: '#FFAA77' } } },
      { operation: 'add_light', light: { type: 'spot', p: { x: 3, y: 2, z: 2, intensity: 100, angle: 45, color: '#7799FF' } } }
    ]) { const r = api.edit(args); if (!r.ok) throw new Error(r.message); }
  });
  await capture('04-point-spot');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
