import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
});

test.describe('@overlays Svelte overlays', () => {
  test('runs a palette command and manages modal focus and toast lifetime', async ({ session }) => {
    test.setTimeout(20_000);
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const before = await page.evaluate(() => (window as any).PM.proj.layers.length as number);
    await page.keyboard.press('Meta+K');
    const palette = page.locator('#palette[data-svelte-overlay-palette="true"]');
    await expect(palette).toHaveCount(1);
    await palette.locator('input[role="combobox"]').fill('New solid');
    await page.keyboard.press('Enter');
    await expect(palette).toHaveCount(0);
    await page.waitForFunction((count) => (window as any).PM.proj.layers.length === count + 1, before);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'solid'))).toBe(
      true,
    );

    await page.evaluate(() => {
      const trigger = document.createElement('button');
      trigger.id = 'overlay-e2e-trigger';
      trigger.textContent = 'Open dialog';
      document.getElementById('app')?.appendChild(trigger);
      trigger.focus();
      (window as any).PM.modal({ title: 'Overlay test', body: 'Live body', actions: [{ label: 'Done', pri: true }] });
    });
    await expect(page.locator('#scrim')).toHaveClass(/\bon\b/);
    await expect(page.locator('.modal[data-svelte-overlay-modal="true"]')).toContainText('Live body');
    await page.keyboard.press('Escape');
    await expect(page.locator('.modal[data-svelte-overlay-modal="true"]')).toHaveCount(0);
    await expect(page.locator('#scrim')).not.toHaveClass(/\bon\b/);
    expect(await page.evaluate(() => document.activeElement?.id)).toBe('overlay-e2e-trigger');

    await page.evaluate(() => (window as any).PM.toast('Overlay ready', 100));
    await expect(page.locator('#toasts .toast')).toHaveText('Overlay ready');
    await expect(page.locator('#toasts .toast')).toHaveCount(0, { timeout: 2_000 });
    expect(diagnostics.pageErrors).toEqual([]);
  });
});

for (const theme of ['light', 'dark'])
  test(`opens a content-sized ${theme} sheet that settles opaque above the viewer`, async ({ session }) => {
    const { page } = session;
    const opening = await page.evaluate((theme) => {
      const P = (window as any).PM;
      document.documentElement.dataset.theme = theme;
      const layer = P.mkLayer('shape', { d: { w: 180, h: 100, color: '#FF4000' } });
      P.proj.layers.push(layer);
      P.selectLayers(layer.id);
      P.bus.emit('layers');
      P.invalidate();
      P.modal({
        title: 'Ordinary sheet',
        body: 'This is ordinary body text.',
        actions: [{ label: 'Done', pri: true }],
      });
      const m = document.querySelector('.modal')!;
      return { background: getComputedStyle(m).backgroundColor };
    }, theme);
    expect(opening.background).toMatch(/^rgb\(/);
    await page.evaluate(async () => {
      await Promise.all(
        document
          .querySelector('.modal')!
          .getAnimations()
          .map((a) => a.finished),
      );
    });
    const geometry = await page.evaluate(() => {
      const m = document.querySelector('.modal')!,
        body = m.querySelector('.mb')!,
        footer = m.querySelector('.mf')!;
      const range = document.createRange();
      range.selectNodeContents(body);
      const b = body.getBoundingClientRect();
      const top = document.elementsFromPoint(b.x + 20, b.y + 10)[0];
      return {
        height: m.getBoundingClientRect().height,
        gap: footer.getBoundingClientRect().top - range.getBoundingClientRect().bottom,
        aboveViewer: !!top?.closest('.modal'),
        opacity: getComputedStyle(m).opacity,
      };
    });
    expect(geometry.aboveViewer).toBe(true);
    expect(geometry.opacity).toBe('1');
    expect(geometry.height).toBeLessThan(170);
    expect(geometry.gap).toBeLessThanOrEqual(24);
    await page.getByRole('button', { name: 'Done', exact: true }).click();
  });
