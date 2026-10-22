import { expect, test } from './helpers/app';

test('Settings section travel settles quickly and manual wheel input owns scrolling', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
  const screen = page.getByRole('dialog', { name: 'Settings', exact: true });
  const nav = screen.getByRole('navigation', { name: 'Settings sections' });
  const scroller = screen.locator('.sg-scroll');
  const storage = nav.getByRole('button', { name: 'Transcription', exact: true });
  const measured = await storage.evaluate(async button => {
    const scroller = document.querySelector<HTMLElement>('#settings-screen .sg-scroll')!;
    const section = document.querySelector<HTMLElement>('[data-settings-page="transcription"]')!;
    const top = Math.min(scroller.scrollHeight - scroller.clientHeight,
      section.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop - parseFloat(getComputedStyle(scroller).paddingTop));
    const start = performance.now(); (button as HTMLElement).click();
    return await new Promise<{ ms: number; delta: number; target: number }>(resolve => {
      const sample = () => { const delta = Math.abs(scroller.scrollTop - top);
        if (delta < 2 || performance.now() - start > 1600) resolve({ ms: performance.now() - start, delta, target: top }); else requestAnimationFrame(sample); };
      requestAnimationFrame(sample);
    });
  });
  console.log('SETTINGS_NAVIGATION', JSON.stringify(measured));
  test.info().annotations.push({ type: 'section-navigation-duration', description: JSON.stringify(measured) });
  expect(measured.delta).toBeLessThan(2);
  expect(measured.ms).toBeLessThan(220);
  await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
  await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBe(0);
  const box = (await scroller.boundingBox())!;
  await page.mouse.move(box.x + 16, box.y + box.height / 2);
  await scroller.evaluate(el => {
    (window as any).__settingsWheel = null;
    el.addEventListener('wheel', event => { (window as any).__settingsWheel = { top: el.scrollTop, event, elapsed: performance.now() - (window as any).__settingsNavStarted }; }, { capture: true, passive: true, once: true });
  });
  await storage.evaluate(button => { (window as any).__settingsNavStarted = performance.now(); (button as HTMLElement).click(); });
  await page.mouse.wheel(0, -240);
  await expect.poll(() => page.evaluate(() => (window as any).__settingsWheel?.top ?? null)).not.toBeNull();
  const before = await page.evaluate(() => (window as any).__settingsWheel.top as number);
  expect(await page.evaluate(() => (window as any).__settingsWheel.elapsed as number)).toBeLessThan(140);
  console.log('WHEEL_EVENT', await page.evaluate(() => ({ top: (window as any).__settingsWheel.top, prevented: (window as any).__settingsWheel.event.defaultPrevented, target: (window as any).__settingsWheel.event.target.className, delta: (window as any).__settingsWheel.event.deltaY, elapsed: (window as any).__settingsWheel.elapsed })));
  // Passive wheel delivery can occur after the compositor has already scrolled.
  // Verify it leaves the requested destination and is not pulled back by navigation.
  await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBeLessThan(measured.target - 50);
  const after = await scroller.evaluate(el => el.scrollTop);
  await page.waitForTimeout(250);
  const final = await scroller.evaluate(el => el.scrollTop);
  expect(final).toBeLessThanOrEqual(after + 3);
  expect(await page.evaluate(() => (window as any).__settingsWheel.event.defaultPrevented)).toBe(false);
  console.log('SETTINGS_WHEEL', JSON.stringify({ before, after, final }));
  expect(await scroller.evaluate(el => getComputedStyle(el).overflowY)).toBe('auto');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Settings retargeting, dismissal, trackpad deltas, and reduced motion remain responsive', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
  const screen = page.getByRole('dialog', { name: 'Settings', exact: true });
  const nav = screen.getByRole('navigation', { name: 'Settings sections' });
  const scroller = screen.locator('.sg-scroll');
  await page.evaluate(() => {
    document.querySelector<HTMLButtonElement>('[data-settings-tab="transcription"]')!.click();
    document.querySelector<HTMLButtonElement>('[data-settings-tab="general"]')!.click();
  });
  await expect(nav.getByRole('button', { name: 'General', exact: true })).toHaveAttribute('aria-current', 'location');
  await expect(screen.getByRole('heading', { name: 'General', exact: true })).toBeInViewport();
  await nav.getByRole('button', { name: 'Transcription', exact: true }).click();
  await screen.press('Escape');
  await expect(screen).toBeHidden();
  await page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
  await expect(screen.getByRole('heading', { name: 'General', exact: true })).toBeInViewport();
  await page.waitForTimeout(200);
  const initial = await scroller.evaluate(el => el.scrollTop);
  const box = (await scroller.boundingBox())!;
  await page.mouse.move(box.x + 16, box.y + box.height / 2);
  for (let n = 0; n < 5; n++) await page.mouse.wheel(0, 12);
  await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBeGreaterThan(initial + 30);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const instant = await nav.getByRole('button', { name: 'Transcription', exact: true }).evaluate(button => {
    (button as HTMLElement).click();
    const scroller = document.querySelector<HTMLElement>('#settings-screen .sg-scroll')!;
    const section = document.querySelector<HTMLElement>('[data-settings-page="transcription"]')!;
    return section.getBoundingClientRect().top - scroller.getBoundingClientRect().top - parseFloat(getComputedStyle(scroller).paddingTop);
  });
  expect(Math.abs(instant)).toBeLessThan(2);
  await screen.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
