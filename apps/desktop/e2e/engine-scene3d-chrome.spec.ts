import path from 'node:path';
import { build } from 'esbuild';
import { expect, test, type LaunchedApp } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

type ChromeState = {
  mode: 'camera' | 'editor';
  gizmoVisible: boolean;
  backing: [number, number] | null;
  frameViewSeq: number | null;
  stateViewSeq: number | null;
  camera: { view: number[]; projection: { kind: string } } | null;
  presenting: boolean;
};

let referenceBundle: Promise<string> | null = null;
async function chromeState(session: LaunchedApp): Promise<ChromeState> {
  referenceBundle ??= build({
    entryPoints: [path.join(__dirname, 'helpers/chrome-reference.ts')],
    bundle: true, write: false, format: 'iife', platform: 'browser', target: 'chrome120',
  }).then(result => result.outputFiles![0]!.text);
  const { page } = session;
  if (!(await page.evaluate(() => typeof (window as any).__chromeReferenceState === 'function')))
    await page.evaluate(await referenceBundle);
  return page.evaluate(() => (window as any).__chromeReferenceState());
}

async function stagePixels(session: LaunchedApp): Promise<Buffer> {
  const rect = await session.page.locator('#stage').evaluate(stage => {
    const r = stage.getBoundingClientRect();
    return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) };
  });
  const base64 = await session.app.evaluate(async ({ BrowserWindow }, bounds) => {
    const window = BrowserWindow.getAllWindows().find(w => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
    return (await window.webContents.capturePage(bounds)).toBitmap().toString('base64');
  }, rect);
  return Buffer.from(base64, 'base64');
}

/** Main's 3D chrome is a gizmo over the composition, including native frames. */
test('native engine frames retain the selection gizmo and follow the editor camera', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const result = PM.Kernel.api('chrome-reference').scene3d.edit({ operation: 'add_object', object: { id: 'chrome-box', source: { primitive: 'box' } } });
    if (!result.ok) throw new Error(result.message);
    PM.selectLayers([]);
  });
  await expect.poll(async () => (await chromeState(session)).presenting, { timeout: 30_000 }).toBe(true);
  const empty = await chromeState(session);
  expect(empty.mode).toBe('camera');
  expect(empty.camera).toBeNull();
  expect(empty.gizmoVisible).toBe(false);
  const before = await stagePixels(session);

  await page.evaluate(() => (window as any).PM.selectLayers(['chrome-box']));
  await expect.poll(async () => (await chromeState(session)).gizmoVisible).toBe(true);
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const selected = await chromeState(session);
  expect(selected.backing?.[0]).toBeGreaterThan(0);
  expect(selected.backing?.[1]).toBeGreaterThan(0);
  const after = await stagePixels(session);
  expect(after.length).toBe(before.length);
  let changed = 0;
  for (let i = 0; i < after.length; i += 4)
    if (after[i] !== before[i] || after[i + 1] !== before[i + 1] || after[i + 2] !== before[i + 2]) changed++;
  expect(changed, 'selection draws visible 3D controls over the native frame').toBeGreaterThan(100);

  await page.evaluate(() => (window as any).PM.Kernel.api('chrome-reference').scene3d.setView('editor'));
  await expect.poll(async () => (await chromeState(session)).camera?.view?.length).toBe(16);
  await expect.poll(async () => {
    const state = await chromeState(session);
    return state.frameViewSeq === state.stateViewSeq && state.mode === 'editor';
  }, { timeout: 30_000 }).toBe(true);
  const editor = await chromeState(session);
  expect(editor.camera!.view.every(Number.isFinite)).toBe(true);
  expect(editor.gizmoVisible).toBe(true);

  await page.evaluate(() => (window as any).PM.Kernel.api('chrome-reference').scene3d.setView('camera'));
  await expect.poll(async () => (await chromeState(session)).camera).toBeNull();
  expect((await chromeState(session)).gizmoVisible).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
