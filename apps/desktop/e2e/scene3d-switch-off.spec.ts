import { mkdir, writeFile } from 'node:fs/promises';
import { chooseNativeMenu, expect, inspectNativeMenu, test } from './helpers/app';

test('3D off keeps artwork flat through grouping, camera animation and editor orbit', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const { page } = session;
  const proof = await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({ name: '3D switch off', w: 640, h: 360, dur: 2 }));
    const api = PM.Kernel.api('switch-off-proof').scene3d;
    const edit = (commands: any) => { const r = PM.Edit.apply(commands); if (!r.ok) throw new Error(r.message); };
    const scene = (args: any) => { const r = api.edit(args); if (!r.ok) throw new Error(r.message); };
    scene({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 6 } } });
    scene({ operation: 'add_object', object: { id: 'model', source: { primitive: 'box' }, p: { x: 1.5 }, material: { p: { color: '#FF7733' } } } });
    const camera = PM.proj.layers.find((l: any) => l.d?.definition === 'powermove.3d.camera');
    edit({ type: 'replace_keyframes', target: camera.id, path: 'position.x', keyframes: [{ time: 0, value: 0 }, { time: 1, value: 3 }] });
    for (const [id, x] of [['plain', 120], ['member', 320], ['parented', 520]] as const) {
      edit({ type: 'add_layer', id, layerType: 'shape', name: id, content: { shape: 'rect', w: 120, h: 80, radius: 0, color: '#38D9FF' }, properties: { 'position.x': x, 'position.y': 150 } });
    }
    const canvas = document.createElement('canvas'); canvas.width = 120; canvas.height = 80;
    canvas.getContext('2d')!.fillRect(0, 0, 120, 80);
    const blob = await new Promise<Blob>(resolve => canvas.toBlob(blob => resolve(blob!)));
    const asset = await PM.assets.add(new File([blob], 'card.png', { type: 'image/png' }), { silent: true });
    edit({ type: 'add_layer', id: 'image', layerType: 'image', content: { asset: asset.id, w: 120, h: 80, fit: 'stretch' }, properties: { 'position.x': 100, 'position.y': 270 } });
    edit({ type: 'add_layer', id: 'title', layerType: 'text', content: { text: 'FLAT', font: 'Arial', size: 50, align: 'center', color: '#FFD966' }, properties: { 'position.x': 320, 'position.y': 270 } });
    api.setExtrusion('title', { enabled: true, depth: 25 });
    edit({ type: 'group_layers', targets: ['member'], name: 'Tilted group' });
    const group = PM.proj.layers.find((l: any) => l.type === 'group');
    edit([{ type: 'set_layer', target: group.id, patch: { threeD: true } }, { type: 'set_property', target: group.id, path: 'rotation.y', value: 55 }]);
    edit({ type: 'add_layer', id: 'rig', layerType: 'null' });
    edit({ type: 'set_layer', target: 'rig', patch: { threeD: true } });
    edit({ type: 'set_layer', target: 'parented', patch: { parent: 'rig' } });
    edit({ type: 'set_property', target: 'rig', path: 'rotation.y', value: 45 });
    const ids = ['plain', 'member', 'parented', 'title', 'image'];
    for (const id of ids) edit({ type: 'set_layer', target: id, patch: { threeD: true } });
    await PM.prepareFrame(0, { mblur: false });
    const render = (id: string, time = 0, editorViewport = false) => [...PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false, inputLayer: id, editorViewport })];
    const on = Object.fromEntries(ids.map(id => [id, render(id)]));
    // Use the user-facing command to turn the layer switches off.
    PM.selectLayers(ids); PM.cmd('toggle3DLayer');
    await PM.prepareFrame(0, { mblur: false });
    const off = Object.fromEntries(ids.map(id => [id, render(id)]));
    const moved = Object.fromEntries(ids.map(id => [id, render(id, 1)]));
    // The model proves the animated camera really changes the scene.
    const modelMoves = JSON.stringify(render('model')) !== JSON.stringify(render('model', 1));
    api.setView('editor'); api.setNavigationMode('orbit');
    const flat = ids.map(id => ({ id, threeD: PM.L(id).threeD,
      cameraStable: JSON.stringify(off[id]) === JSON.stringify(moved[id]),
      visiblePixels: off[id].filter((value: number, index: number) => index % 4 === 3 && value > 0).length,
      changedOnDisable: JSON.stringify(on[id]) !== JSON.stringify(off[id]) }));
    // A 2D member must match the ordinary affine group transform, even while
    // its group retains authored depth and Y rotation.
    const grouped = render('member');
    edit({ type: 'set_layer', target: group.id, patch: { threeD: false } });
    const groupOff = render('member');
    edit({ type: 'set_layer', target: group.id, patch: { threeD: true } });
    PM.selectLayers(['member']); PM.invalidate();
    const depth = api.setExtrusion('title', { depth: 45 });
    if (!depth.ok) throw new Error(depth.message);
    return { flat, modelMoves, groupStable: JSON.stringify(grouped) === JSON.stringify(groupOff),
      depthEditStaysFlat: PM.L('title').threeD === false };
  });
  console.log('3D off proof', JSON.stringify(proof));
  // Exercise the actual editor orbit tool as well as the animated Camera layer.
  const orbit = page.getByRole('combobox', { name: '3D navigation', exact: true });
  await orbit.click(); await page.getByRole('option', { name: 'Orbit', exact: true }).click();
  const sample = () => page.evaluate(() => {
    const PM = (window as any).PM;
    return Object.fromEntries(['plain', 'member', 'parented', 'title', 'image', 'model'].map(id => [id,
      [...PM.GL.renderToPixels(0, 160, 90, { transparent: true, mblur: false, inputLayer: id, editorViewport: true })].join(',')]));
  });
  const beforeOrbit = await sample();
  const stage = (await page.locator('#stage').boundingBox())!;
  await page.mouse.move(stage.x + stage.width * .7, stage.y + stage.height * .6);
  await page.mouse.down(); await page.mouse.move(stage.x + stage.width * .7 + 80, stage.y + stage.height * .6 - 30, { steps: 8 }); await page.mouse.up();
  const afterOrbit = await sample();
  if (process.env.PM_SHOT_LABEL) {
    const dir = '/tmp/pm-shots/3d-off'; await mkdir(dir, { recursive: true });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/${process.env.PM_SHOT_LABEL}-switch-off-orbit.png` });
    const png = await page.evaluate(() => {
      const PM = (window as any).PM;
      const pixels = PM.GL.renderToPixels(0, 640, 360, { transparent: true, mblur: false, inputLayer: 'member', editorViewport: true });
      const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
      canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels), 640, 360), 0, 0);
      return canvas.toDataURL('image/png').split(',')[1];
    });
    await writeFile(`${dir}/${process.env.PM_SHOT_LABEL}-member-off.png`, Buffer.from(png, 'base64'));
  }
  expect(proof.modelMoves).toBe(true);
  for (const layer of proof.flat) {
    expect(layer.threeD, layer.id).toBe(false);
    expect(layer.visiblePixels, layer.id).toBeGreaterThan(20);
    expect(layer.cameraStable, layer.id).toBe(true);
    expect(layer.changedOnDisable, layer.id).toBe(true);
    expect(afterOrbit[layer.id], layer.id).toBe(beforeOrbit[layer.id]);
  }
  expect(afterOrbit.model).not.toBe(beforeOrbit.model);
  expect(proof.groupStable, '2D member must ignore group depth and tilt').toBe(true);
  expect(proof.depthEditStaysFlat).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('timeline, layer menu and command agree on artwork switches; model-only groups have no switch', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const group = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('switch-ui-proof').scene3d;
    api.edit({ operation: 'add_object', object: { id: 'model', source: { primitive: 'box' } } });
    PM.Edit.apply({ type: 'group_layers', targets: ['model'], name: 'Models' });
    const group = PM.proj.layers.find((l: any) => l.type === 'group');
    PM.Edit.apply({ type: 'add_layer', id: 'art', layerType: 'shape' });
    PM.bus.emit('draw:timeline');
    return group.id;
  });
  const enabled = (id: string) => page.evaluate(id => !!(window as any).PM.L(id).threeD, id);
  const switchPoint = (id: string) => page.evaluate(id => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    PM.bus.emit('draw:timeline');
    const index = timeline.rows.findIndex((row: any) => row.kind === 'layer' && row.L.id === id);
    const rect = timeline.cv.getBoundingClientRect();
    return { x: rect.left + timeline.gut - 110 - 27, y: rect.top + timeline.ruler + (index + .5) * timeline.row - timeline.scrollY };
  }, id);
  const art = await switchPoint('art');
  await page.mouse.click(art.x, art.y); expect(await enabled('art')).toBe(true);
  await page.mouse.click(art.x, art.y); expect(await enabled('art')).toBe(false);
  const openMenu = (id: string) => page.evaluate(id => {
    const PM = (window as any).PM; PM.selectLayers([id]);
    PM.showLayerMenu(PM.L(id), { clientX: 300, clientY: 300 });
  }, id);
  await chooseNativeMenu(session, '3D Layer', () => openMenu('art'));
  expect(await enabled('art')).toBe(true);
  await page.evaluate(() => (window as any).PM.cmd('toggle3DLayer'));
  expect(await enabled('art')).toBe(false);
  const modelGroup = await switchPoint(group);
  await page.mouse.move(modelGroup.x, modelGroup.y);
  expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').hoverSwitch)).toBeNull();
  await page.mouse.click(modelGroup.x, modelGroup.y);
  expect(await enabled(group)).toBe(true);
  const menu = await inspectNativeMenu(session, () => openMenu(group));
  expect(menu.some(item => item.label === '3D Layer')).toBe(false);
  // A mixed selection still toggles eligible artwork instead of failing the
  // entire operation because an always-3D layer was also selected.
  await page.evaluate(group => {
    const PM = (window as any).PM; PM.selectLayers([group, 'model', 'art']); PM.cmd('toggle3DLayer');
  }, group);
  expect(await enabled('art')).toBe(true);
  expect(await enabled(group)).toBe(true);
  expect(await enabled('model')).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
