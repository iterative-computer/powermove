import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('library previews contain tall panel content without changing the live panel', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const el = document.createElement('div');
    el.className = 'panel';
    el.style.cssText = 'display:flex;flex-direction:column;width:360px;height:220px';
    el.innerHTML = '<div style="flex:none;height:32px">Tall panel</div><div class="fit-body" style="flex:1;min-height:0;overflow:auto"><div style="height:600px">Panel content</div><div class="fit-last" style="height:32px">Last control</div></div><div class="fit-footer" style="height:48px;flex:none">Footer</div>';
    document.body.appendChild(el);
    PM.PANELS['fit-fixture'] = { title: 'Tall panel', library: { width:360, height:220 } };
    PM.panelInst['fit-fixture'] = { el };
    PM.LibraryUI.open();
  });
  const card = page.locator('[data-panel-id="fit-fixture"]');
  await expect(card).toBeVisible();
  const geometry = await card.evaluate(card => {
    const clone = card.querySelector<HTMLElement>('.library-clone')!;
    const body = clone.querySelector<HTMLElement>('.fit-body')!;
    const last = clone.querySelector('.fit-last')!.getBoundingClientRect();
    const footer = clone.querySelector('.fit-footer')!.getBoundingClientRect();
    const bounds = card.getBoundingClientRect();
    return { overflow: body.scrollHeight-body.clientHeight, last: last.bottom, footer:footer.top, bottom:footer.bottom, card:bounds.bottom };
  });
  expect(geometry.overflow).toBeLessThanOrEqual(1);
  expect(geometry.last).toBeLessThanOrEqual(geometry.footer + 1);
  expect(geometry.bottom).toBeLessThanOrEqual(geometry.card + 1);
  expect(await page.evaluate(() => (window as any).PM.panelInst['fit-fixture'].el.clientHeight)).toBe(220);
});

test('built-in Library previews fit their scrollable content', async ({ session }) => {
  const { page } = session;
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await page.evaluate(() => (window as any).PM.LibraryUI.open());
  const clipped = await page.locator('.library-clone').evaluateAll(clones => clones.flatMap(clone => {
    return [...clone.querySelectorAll<HTMLElement>('*')].filter(node =>
      /^(auto|scroll|hidden)$/.test(getComputedStyle(node).overflowY) && !['absolute', 'fixed'].includes(getComputedStyle(node).position) && !node.querySelector('canvas') && node.clientHeight > 0 && node.scrollHeight > node.clientHeight + 1
    ).map(node => ({ panel: clone.closest<HTMLElement>('[data-panel-id]')!.dataset.panelId, class:node.className, overflow:node.scrollHeight-node.clientHeight }));
  }));
  expect(clipped).toEqual([]);
  await page.screenshot({ path: '/private/tmp/powermove-library-fit.png' });
});
