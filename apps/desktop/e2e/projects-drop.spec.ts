import { readFile } from 'node:fs/promises';
import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';

for (const existingProject of [false, true]) {
  test(`dropping an image on Projects opens a new project ${existingProject ? 'without changing the previous project' : 'from a fresh home screen'}`, async ({ session }) => {
    if (existingProject) await session.openEditor();
    const before = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.isHomeProject()) {
        PM.Edit.apply({ type: 'add_layer', layerType: 'text', name: 'Keep me', content: { text: 'Original' } });
        PM.setTime(3);
      }
      PM.ProjectsScreen.show();
      return { id: PM.proj.id, layers: PM.proj.layers.length };
    });
    const bytes = [...await readFile(fixturePath('still-red.png'))];
    await session.page.evaluate(bytes => {
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(new File([new Uint8Array(bytes)], 'Dropped image.png', { type: 'image/png' }));
      document.querySelector('.ps-grid')!.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    }, bytes);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Dropped image');
    await expect(session.page.locator('#projects-screen')).not.toHaveClass(/\bon\b/);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
    const state = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers[0];
      const asset = PM.assets.get(layer.d.asset);
      return { id: PM.proj.id, w: PM.proj.w, h: PM.proj.h, from: layer.from,
        name: layer.name, assetW: asset.w, assetH: asset.h, assets: Object.keys(PM.proj.assets).length };
    });
    expect(state.id).not.toBe(before.id);
    expect(state).toMatchObject({ from: 0, name: 'Dropped image.png', assets: 1 });
    expect(state.w).toBe(state.assetW);
    expect(state.h).toBe(state.assetH);
    if (existingProject) {
      await session.page.evaluate(id => (window as any).PM.Tabs.activate(id), before.id);
      expect(await session.page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.name))).toEqual(['Keep me']);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

test('dropping a project opens it once and keeps the previous composition intact', async ({ session }) => {
  await session.openEditor();
  const before = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.ProjectsScreen.show();
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(new File([JSON.stringify({ proj: PM.mkProject({ name: 'Dropped project', w: 640, h: 360 }) })], 'Dropped.pmv'));
    const id = PM.proj.id;
    document.querySelector('.ps-grid')!.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    return id;
  });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Dropped project');
  await expect(session.page.locator('#projects-screen')).not.toHaveClass(/\bon\b/);
  await expect(session.page.locator('#tabs .project-doc')).toHaveCount(2);
  expect(await session.page.evaluate(() => (window as any).PM.proj.id)).not.toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('unsupported and empty drops leave Projects and the current project unchanged', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const id = PM.proj.id;
    PM.ProjectsScreen.show();
    for (const unsupported of [false, true]) {
      const dataTransfer = new DataTransfer();
      if (unsupported) dataTransfer.items.add(new File(['hello'], 'notes.txt', { type: 'text/plain' }));
      document.querySelector('.ps-grid')!.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    }
    await PM.app.importQueue;
    return { sameProject: PM.proj.id === id, home: PM.ProjectsScreen.isOpen, layers: PM.proj.layers.length };
  });
  expect(result).toEqual({ sameProject: true, home: true, layers: 0 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
