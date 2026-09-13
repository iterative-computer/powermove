import { test, expect } from './helpers/app';
import { cp, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('panel reload keeps the project and canvas while simplifying editor controls', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'); return viewer?.ov && PM?.GL?.gl; });
  await page.evaluate(() => (window as any).PM.Kernel.loader.builtinsReady);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const audio = PM.mkLayer('audio', { name: 'Soundtrack' });
    PM.proj.layers = [audio]; PM.ProjectIndex.invalidate(); PM.touch(); PM.selectLayers(audio.id);
    (window as any).__beforeCleanup = { project: PM.serialize(), canvas: PM.GL.canvas };
  });
  await mkdir(path.join(session.userData, 'extensions'), { recursive: true });
  const fork = path.join(session.userData, 'extensions/interface-cleanup');
  for (const panel of ['timeline', 'inspector', 'viewer']) {
    await cp(path.resolve('src/extensions', panel), path.join(fork, panel), { recursive: true });
  }
  await writeFile(path.join(fork, 'index.ts'), `import timeline from './timeline/index'; import inspector from './inspector/index'; import viewer from './viewer/index'; export default function activate(api) { timeline(api); inspector(api); viewer(api); }`);
  await writeFile(path.join(fork, 'manifest.json'), JSON.stringify({ id: 'interface-cleanup', name: 'Interface cleanup', version: '1.0.0', apiVersion: 1, entry: 'index.ts', replaces: ['timeline', 'inspector', 'viewer'] }));
  await page.waitForFunction(() => (window as any).PM.Kernel.loader.activeIds().includes('interface-cleanup'));
  await expect(page.getByRole('combobox', { name: 'Source', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /Animate Visible|Remove animation from Visible/ })).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Preview frame rate', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Cache work area preview', exact: true })).toHaveCount(0);
  await expect(page.getByRole('searchbox', { name: 'Search timeline' })).toHaveCount(0);
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, before = (window as any).__beforeCleanup;
    const head = document.querySelector('#tl-head')!.getBoundingClientRect();
    const canvas = document.querySelector('#tl-canvas')!.getBoundingClientRect();
    const quality = document.querySelector('#preview-controls select')!.getBoundingClientRect();
    const zoom = document.querySelector('#composition-zoom')!.getBoundingClientRect();
    return { sameProject: PM.serialize() === before.project, sameCanvas: PM.GL.canvas === before.canvas,
      inlineRuler: Math.abs(head.top - canvas.top) < 1, compact: head.width < canvas.width,
      adjacentMenus: zoom.left - quality.right >= 0 && zoom.left - quality.right < 20,
      alignedMenus: quality.top === zoom.top && quality.height === zoom.height };
  })).toEqual({ sameProject: true, sameCanvas: true, inlineRuler: true, compact: true, adjacentMenus: true, alignedMenus: true });
  await page.getByRole('button', { name: 'Visible', exact: true }).click();
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].on)).toBe(false);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].on)).toBe(true);
  await page.screenshot({ path: '/private/tmp/pm-interface-cleanup.png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
