import { expect, test } from './helpers/app';

test('dragging selected media into a folder moves the batch and one Undo restores it', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const fixture = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 16;
    const files: File[] = [];
    for (let index = 0; index < 3; index++) {
      const context = canvas.getContext('2d')!;
      context.fillStyle = ['#ff0000', '#00ff00', '#0000ff'][index]!;
      context.fillRect(0, 0, 16, 16);
      const encoded = canvas.toDataURL('image/png').split(',')[1]!;
      const bytes = Uint8Array.from(atob(encoded), character => character.charCodeAt(0));
      files.push(new File([bytes], `Folder-${String.fromCharCode(65 + index)}.png`, { type: 'image/png' }));
    }
    await PM.importFiles(files, { placement: null });
    PM.proj.assetFolders = { destination: { id: 'destination', name: 'Destination', parent: null } };
    PM.bus.emit('assets'); PM.touch(); PM.invalidate(); PM.hist.clear();
    return { ids: Object.keys(PM.proj.assets), folder: 'destination' };
  });
  const cards = page.locator('.asset-card[data-asset-id]');
  const first = page.locator(`[data-asset-id="${fixture.ids[0]}"]`);
  const second = page.locator(`[data-asset-id="${fixture.ids[1]}"]`);
  const folder = page.locator(`[data-folder-id="${fixture.folder}"]`);
  await expect(cards).toHaveCount(3);
  await first.locator('.asset-copy').click();
  await second.locator('.asset-copy').click({ modifiers: ['Meta'] });
  await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(2);
  await expect(folder).toBeVisible();
  // Exercise the real pointerdown and DOM drop handlers without a hidden OS drag loop.
  await first.hover();
  await page.mouse.down();
  await page.evaluate(({ id, folder }) => {
    const card = document.querySelector(`[data-asset-id="${id}"]`)!;
    const destination = document.querySelector(`[data-folder-id="${folder}"]`)!;
    const dataTransfer = new DataTransfer();
    card.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer }));
    destination.dispatchEvent(new DragEvent('dragover', { bubbles: true, dataTransfer }));
    destination.dispatchEvent(new DragEvent('drop', { bubbles: true, dataTransfer }));
    card.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer }));
  }, { id: fixture.ids[0], folder: fixture.folder });
  await page.mouse.up();

  const locations = () => page.evaluate(ids => ids.map(id => (window as any).PM.proj.assets[id].folder ?? null), fixture.ids);
  await expect.poll(locations).toEqual([fixture.folder, fixture.folder, null]);
  await expect(cards).toHaveCount(1);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  await expect.poll(locations).toEqual([null, null, null]);
  await expect(cards).toHaveCount(3);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
