import { expect, test } from './helpers/app';

test('the error toast matches the status toast it stacks with', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  // The error is pushed first: a routine status replaces its peers, but never an error.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.toast(new Error("ENOSPC: no space left on device, write '/Users/me/Movies/render.mov'"), 8000);
    PM.toast('Imported Hero shot.mp4', 8000, { error: false });
  });
  const stack = page.locator('.toastwrap');
  const error = page.locator('.toast[data-toast-error="true"]');
  await expect(error).toBeVisible();
  await expect(page.locator('.toast:not([data-toast-error])')).toBeVisible();

  // Both kinds share one icon gutter, so their text columns line up.
  const columns = await stack.evaluate(el => [...el.querySelectorAll<HTMLElement>('.toast')]
    .map(toast => {
      const shell = toast.getBoundingClientRect();
      const icon = toast.querySelector('.toast-icon')!.getBoundingClientRect();
      const text = toast.querySelector(':scope > .body > strong, :scope > span:not(.toast-icon)')!.getBoundingClientRect();
      return { icon: Math.round(icon.left - shell.left), text: Math.round(text.left - shell.left) };
    }));
  expect(columns[0].icon).toBe(columns[1].icon);
  expect(columns[0].text).toBe(columns[1].text);

  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
    await stack.screenshot({ path: info.outputPath(`toast-error-${theme}.png`) });
  }

  await error.getByRole('button', { name: 'Technical details' }).click();
  await expect(error.locator('pre')).toContainText('ENOSPC');
  await stack.screenshot({ path: info.outputPath('toast-error-details.png') });
  await error.getByRole('button', { name: 'Dismiss notification' }).click();
  await expect(error).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
