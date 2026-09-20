import path from 'node:path';
import { expect, launchApp, repoRoot, test } from './helpers/app';

test('connection setup owns the empty panel and stays usable when resized', async ({}, testInfo) => {
  const session = await launchApp({ env: {
    CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
    POWERMOVE_FAKE_CHATGPT_STATUS: 'disconnected'
  } });
  try {
    await session.openEditor();
    const { page } = session;
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.theme.apply('dark');
      PM.SpatialAssistant.open();
    });
    const panel = page.locator('#panel-agent');
    const gate = panel.locator('.agent-connect-gate');
    await expect(gate).toBeVisible();
    await panel.screenshot({ path: testInfo.outputPath('connection-initial.png') });
    await expect(panel.locator('.agent-welcome')).toBeHidden();
    await expect(panel.locator('[aria-label="Message composer"]')).toBeHidden();
    for (const width of [240, 320, 480]) {
      await panel.evaluate((el, width) => {
        Object.assign((el as HTMLElement).style, { width: `${width}px`, maxWidth: `${width}px`, minWidth: '0' });
        (el.closest('.dock') as HTMLElement).style.flex = `0 0 ${width}px`;
      }, width);
      for (const provider of ['chatgpt', 'compatible']) {
        await gate.getByRole('button', { name: provider === 'chatgpt' ? 'Codex' : 'API / local', exact: true }).click();
        await expect(gate.getByRole('button', { name: provider === 'chatgpt' ? 'Connect ChatGPT' : 'Open settings', exact: true })).toBeInViewport();
        const overflow = await gate.evaluate(root => [root, ...root.querySelectorAll('*')]
          .filter(el => el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 1)
          .map(el => el.className));
        expect(overflow, `${provider} at ${width}px`).toEqual([]);
        await panel.screenshot({ path: testInfo.outputPath(`connection-${provider}-${width}.png`) });
      }
    }
    await page.evaluate(() => { (window as any).PM.theme.apply('light'); });
    await panel.screenshot({ path: testInfo.outputPath('connection-light.png') });
    await panel.evaluate(el => (el as HTMLElement).style.setProperty('--set-panel-height', '220px'));
    const settingsButton = gate.getByRole('button', { name: 'Open settings', exact: true });
    await settingsButton.scrollIntoViewIfNeeded();
    await expect(settingsButton).toBeInViewport();
    await settingsButton.click();
    await expect(page.getByRole('dialog', { name: 'Settings', exact: true })).toBeVisible();
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
