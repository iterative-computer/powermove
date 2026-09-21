import { expect, test } from './helpers/app';

test('notification close buttons stay at the trailing edge and dismiss by mouse or keyboard', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  const cases = [
    { message: 'Could not open the panel window', options: {} },
    { message: 'Could not import missing.mov', options: {} },
    { message: 'Could not import ' + 'very-long-filename-'.repeat(30) + '.mov', options: {} },
    { message: 'Update failed', options: { source: { id: 'timeline', name: 'Timeline' } } },
    { message: 'Update failed', options: { corner: 'top-right', action: true } },
    { message: 'Ready', options: { kind: 'status', sticky: true, dismissible: true } }
  ];
  for (const [index, example] of cases.entries()) {
    await page.evaluate(({ message, options }) => {
      const PM = (window as any).PM;
      PM.theme.apply('dark');
      (window as any).__toastDismissals = 0;
      PM.toast(message, 8000, {
        ...options,
        action: options.action ? { label: 'Retry', run: () => {} } : undefined,
        onDismiss: () => { (window as any).__toastDismissals++; }
      });
    }, example);
    const toast = page.locator('.toast').filter({ has: page.getByRole('button', { name: 'Dismiss notification' }) });
    const close = toast.getByRole('button', { name: 'Dismiss notification' });
    await expect(close).toBeVisible();
    if (index === 0) await toast.screenshot({ path: info.outputPath('panel-error-close.png') });
    const checkPlacement = async () => {
      const layout = await toast.evaluate(el => {
        const shell = el.getBoundingClientRect();
        const button = el.querySelector('[aria-label="Dismiss notification"]')!.getBoundingClientRect();
        return {
          rightInset: shell.right - button.right,
          padding: parseFloat(getComputedStyle(el).paddingRight),
          overflow: el.scrollWidth - el.clientWidth
        };
      });
      expect(Math.abs(layout.rightInset - layout.padding)).toBeLessThanOrEqual(1);
      expect(layout.overflow).toBeLessThanOrEqual(1);
    };
    await checkPlacement();
    const disclosure = toast.getByRole('button', { name: 'Technical details' });
    if (await disclosure.count()) {
      await disclosure.click();
      await expect(toast.locator('pre')).toBeVisible();
      await checkPlacement();
    }
    if (index % 2) {
      await close.focus();
      await page.keyboard.press('Enter');
    } else {
      await close.click();
    }
    await expect(toast).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__toastDismissals)).toBe(1);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

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
