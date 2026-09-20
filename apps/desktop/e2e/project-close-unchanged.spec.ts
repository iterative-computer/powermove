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
  expect(await session.page.evaluate(() => (window as any).PM.prepareToClose())).toBe(true);
  expect(await session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(0);

  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Edited close test' }),
    }));
    PM.proj.bg = '#ff0000';
    PM.autosave();
  });
  expect(await session.page.evaluate(() => (window as any).PM.prepareToClose())).toBe(false);
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(1);
  expect(await session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Edited close test');
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
});
