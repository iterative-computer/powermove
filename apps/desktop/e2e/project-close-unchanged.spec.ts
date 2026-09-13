import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('closing an unchanged local project skips the save prompt, while edits still prompt', async ({ session }) => {
  await session.app.evaluate(({ dialog }) => {
    (globalThis as any).__closePrompts = 0;
    dialog.showMessageBox = async () => {
      (globalThis as any).__closePrompts++;
      return { response: 1, checkboxChecked: false };
    };
  });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Untouched close test' }),
    }));
  });
  await session.page.getByRole('button', { name: 'Close Untouched close test', exact: true }).click();
  await expect(session.page.getByRole('button', { name: 'Close Untouched close test', exact: true })).toHaveCount(0);
  expect(await session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(0);

  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Edited close test' }),
    }));
    PM.proj.bg = '#ff0000';
    PM.autosave();
  });
  await session.page.getByRole('button', { name: 'Close Edited close test', exact: true }).click();
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(1);
  await expect(session.page.getByRole('button', { name: 'Close Edited close test', exact: true })).toBeVisible();
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
});
