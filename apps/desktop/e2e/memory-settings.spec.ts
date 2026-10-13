import { expect, launchApp, test } from './helpers/app';

test('Settings › Memory lists the project notes and switches agent access on and off', async ({}, info) => {
  const session = await launchApp();
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj.memory = [
        { id: 'g', kind: 'goal', text: '30s product teaser for new customers', author: 'user', createdAt: 1, updatedAt: 1 },
        { id: 'c', kind: 'constraint', text: 'Brand blue only', author: 'user', createdAt: 2, updatedAt: 2 },
        { id: 'd', kind: 'decision', text: 'Cut the intro to 2s', why: 'The voiceover starts at 1.8s', author: 'agent', createdAt: 3, updatedAt: 3 },
        { id: 'r', kind: 'rejected', text: 'Zoom transition into the demo', why: 'Looked wrong against the UI footage', author: 'agent', createdAt: 4, updatedAt: 4 },
        { id: 'o', kind: 'open', text: 'Waiting on the logo file', author: 'agent', createdAt: 5, updatedAt: 5 }
      ];
    });
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await settings.getByRole('button', { name: 'Memory', exact: true }).click();

    const panel = settings.getByRole('region', { name: 'Memory', exact: true });
    await expect(panel).toContainText('30s product teaser for new customers');
    await expect(panel).toContainText('The voiceover starts at 1.8s');
    await expect(panel).toContainText('Rejected ideas');
    const reveal = () => panel.evaluate(el => el.scrollIntoView({ block: 'start', behavior: 'instant' }));
    await reveal();
    const toggle = panel.getByRole('button', { name: 'Use project memory', exact: true });
    await expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await settings.screenshot({ path: info.outputPath('memory-on.png') });

    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    expect(await page.evaluate(() => (window as any).PM.proj.memoryEnabled)).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.proj.memory.length)).toBe(5);
    await reveal();
    await settings.screenshot({ path: info.outputPath('memory-off.png') });

    await toggle.click();
    expect(await page.evaluate(() => (window as any).PM.proj.memoryEnabled)).toBeUndefined();

    // A note an agent adds while Settings is open appears without reopening it.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj.memory = [...PM.proj.memory, { id: 'f', kind: 'feedback', text: 'Wants it punchier', author: 'user', createdAt: 6, updatedAt: 6 }];
      PM.bus.emit('memory');
    });
    await expect(panel).toContainText('Wants it punchier');
  } finally {
    await session.close();
  }
});
