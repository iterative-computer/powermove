import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';

test.beforeEach(async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Reliability proof', w: 640, h: 360, dur: 5, fps: 30 });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    const result = PM.Edit.apply({ type: 'add_layer', layerType: 'solid', name: 'Animated square', content: { w: 100, h: 100, color: '#ff6b1a' } });
    if (!result.ok) throw new Error(result.message);
    PM.setTime(0);
  });
});

test('inspector keyframes animate, linked scale edits undo together, and handles extend outside video', async ({ session }, info) => {
  const { page } = session;
  const x = page.getByRole('spinbutton', { name: 'Position X', exact: true });
  await page.getByRole('button', { name: 'Animate Position X', exact: true }).click();
  await page.evaluate(() => (window as any).PM.setTime(2));
  await x.click(); await x.fill('500'); await x.press('Enter');
  const animation = await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.firstSel();
    return { keys: layer.p['position.x'].kf.map((k: any) => [k.t, k.v]), id: layer.id };
  });
  expect(animation.keys).toHaveLength(2);
  expect(animation.keys[1]).toEqual([2, 500]);
  for (const time of [0, 1, 2, 0]) {
    await page.evaluate(t => (window as any).PM.setTime(t), time);
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM, L = PM.firstSel();
      return Math.round(PM.worldMatrix(L, PM.time)[4] - PM.ev(L, 'position.x', PM.time));
    })).toBe(0);
  }
  await page.getByRole('button', { name: 'Link Scale X and Y', exact: true }).click();
  const sx = page.getByRole('spinbutton', { name: 'Scale X', exact: true });
  await sx.click(); await sx.fill('150'); await sx.press('Enter');
  await expect(page.getByRole('spinbutton', { name: 'Scale Y', exact: true })).toHaveValue('150%');
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect(sx).toHaveValue('100%');
  await expect(page.getByRole('spinbutton', { name: 'Scale Y', exact: true })).toHaveValue('100%');
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'set_property', target: PM.firstSel().id, path: 'position.x', value: -25, mode: 'keyframe', time: 0, preserveHandEdits: false });
    PM.Viewer.fit = false; PM.Viewer.zoom = .6; PM.Viewer.pan = [0, 0]; PM.Viewer.layout();
  });
  await expect.poll(() => page.evaluate(() => {
    const stage = document.getElementById('stage')!.getBoundingClientRect();
    const overlay = document.getElementById('overlay')!.getBoundingClientRect();
    return Math.abs(stage.width - overlay.width) < 1 && document.getElementById('overlay')!.parentElement?.id === 'stage';
  })).toBe(true);
  await page.screenshot({ path: info.outputPath('out-of-frame-selection.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('cancelled Save never reports a saved file; successful Save writes a reopenable project', async ({ session }, info) => {
  const { page, app } = session;
  await app.evaluate(({ dialog }) => { dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' }); });
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    clearTimeout(PM.app.saveTimer); PM.app.dirty = true;
    (window as any).__saveToasts = [];
    const toast = PM.toast; PM.toast = (text: string, ...args: any[]) => { (window as any).__saveToasts.push(text); toast(text, ...args); };
    await PM.saveProject();
  });
  expect(await page.evaluate(() => (window as any).__saveToasts)).not.toContain('Project downloaded');
  expect(await page.evaluate(() => (window as any).PM.app.dirty)).toBe(true);
  const file = path.join(session.userData, 'roundtrip.pmv');
  await app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
  const original = await page.evaluate(async () => {
    const PM = (window as any).PM, layer = PM.firstSel();
    PM.Edit.apply([
      { type: 'set_layer', target: layer.id, patch: { scaleLinked: true } },
      { type: 'set_property', target: layer.id, path: 'position.x', value: 100, time: 0, mode: 'keyframe', preserveHandEdits: false },
      { type: 'set_property', target: layer.id, path: 'position.x', value: 300, time: 2, mode: 'keyframe', preserveHandEdits: false }
    ]);
    await PM.saveProject(); return JSON.parse(PM.serialize());
  });
  const savedBytes = await readFile(file);
  const saved = decodeProjectContainer(savedBytes).document;
  expect(saved.proj).toEqual(original.proj);
  await page.evaluate(async bytes => {
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'delete_layers', targets: PM.proj.layers.map((L: any) => L.id) });
    await PM.importFiles([new File([new Uint8Array(bytes)], 'roundtrip.pmv', { type: 'application/octet-stream' })]);
  }, Array.from(savedBytes));
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].p['position.x'].kf.map((k: any) => [k.t, k.v]))).toEqual([[0, 100], [2, 300]]);
  await page.screenshot({ path: info.outputPath('saved-project.png') });
  await page.evaluate(async () => { await new Promise(resolve => setTimeout(resolve, 800)); await (window as any).powermove.store.flush(); });
  await session.relaunch(); // Only the isolated temporary test app, never the working app.
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  expect(await session.page.evaluate(() => (window as any).PM.proj.layers[0].scaleLinked)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
