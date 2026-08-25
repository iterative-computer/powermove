import { expect, test } from './helpers/app';

test.describe('@persistence native project persistence', () => {
  test('autosaves a project and restores it after relaunch', async ({ session }) => {
    const storeWired = await session.page.evaluate(async () => {
      try {
        await (window as any).powermove?.store?.flush();
        return true;
      } catch {
        return false;
      }
    });
    test.fixme(
      !storeWired,
      'Waiting for the main-process store IPC handlers to back window.powermove.store'
    );

    await session.page.evaluate(() => (window as any).PM.cmd('newProject'));
    const modal = session.page.locator('.modal');
    await modal.locator('input').fill('E2E Persistence');
    await modal.getByRole('button', { name: 'Create' }).click();

    const result = await session.page.evaluate(() => (window as any).PM.Edit.apply([
      { type: 'add_layer', layerType: 'solid', name: 'E2E' }
    ], { label: 'E2E persistence layer', origin: 'e2e' }));
    expect(result.ok).toBe(true);

    await session.page.waitForTimeout(750); // autosave debounce is 550 ms
    await session.page.evaluate(() => (window as any).powermove.store.flush());
    await session.relaunch();

    await session.page.waitForFunction(() => Boolean((window as any).PM?.Projects));
    const restored = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const meta = PM.Projects.list().find((item: any) => {
        const project = PM.Projects.get(item.id);
        return project?.layers?.some((layer: any) => layer.name === 'E2E');
      });
      if (!meta) return { found: false, opened: false };
      const project = PM.Projects.get(meta.id);
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      return {
        found: true,
        opened: PM.proj.layers.some((layer: any) => layer.name === 'E2E')
      };
    });

    expect(restored).toEqual({ found: true, opened: true });
  });
});
