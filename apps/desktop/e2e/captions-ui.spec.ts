import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

/* Review screenshots go outside the repo; nothing here is committed output. */
const SHOTS = process.env.PM_CAPTION_SHOTS || '/tmp/pm-captions';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

async function seed(page: import('playwright').Page) {
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Captions UI', w: 1920, h: 1080, dur: 12 });
    const backdrop = PM.mkLayer('solid', { name: 'Backdrop', d: { color: '#34495E' } }, project);
    project.layers = [backdrop];
    PM.replaceProject(project);
    const words = (text: string, start: number, step: number) => text.split(' ').map((word, index) => ({ text: word, start: start + index * step, end: start + index * step + step * .9 }));
    const cues = [
      { start: .4, end: 2.2, text: 'Every frame tells a story.', words: words('Every frame tells a story.', .4, .36) },
      { start: 2.4, end: 4.6, text: 'Captions make every word heard.', words: words('Captions make every word heard.', 2.4, .44) },
      { start: 5, end: 7.2, text: 'Even in a quiet room.', words: words('Even in a quiet room.', 5, .44) },
      { start: 7.6, end: 10.4, text: 'Generated, styled and exported in one place.' }
    ];
    PM.Edit.apply({ type: 'add_captions', name: 'English', language: 'en', cues, style: { preset: 'spotlight' } }, { label: 'Captions', origin: 'command' });
    PM.Edit.apply({ type: 'add_captions', name: 'Français', language: 'fr', cues: [{ start: .4, end: 4, text: 'Chaque image raconte une histoire.' }], style: { preset: 'whisper' }, select: false });
    const english = PM.proj.layers.find((layer: any) => layer.name === 'English');
    PM.selectLayers([english.id]);
    PM.Captions.select(english.id, [english.d.cues[1].id]);
    PM.WS.mutate((workspace: any) => { if (!PM.Layout.hasPanel(workspace, 'captions')) PM.Layout.addPanel(workspace, 'captions', 'right'); });
    PM.setTime(3.3, { force: true });
    PM.Kernel.services.get('timeline').frameView();
    return english.id;
  });
}

async function settle(page: import('playwright').Page) {
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await page.waitForTimeout(250);
}

for (const scheme of ['light', 'dark'] as const) {
  test(`captions surfaces in ${scheme} mode`, async ({ session }) => {
    const { page } = session;
    await mkdir(SHOTS, { recursive: true });
    const id = await seed(page);
    await page.evaluate((scheme) => (window as any).PM.Kernel.setScheme(scheme), scheme);
    await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme === 'dark')).toBe(scheme === 'dark');

    // Inspector: presets and style rows for the selected captions layer.
    const inspector = page.locator('[data-inspector-layer]');
    await expect(inspector).toHaveAttribute('data-inspector-layer', id);
    await expect(page.locator('.caption-preset')).toHaveCount(6);
    await expect(page.locator('.caption-preset[aria-checked="true"]')).toContainText('Spotlight');

    // Captions panel: a quiet list of the cues, the selected one marked.
    const rows = page.locator('.captions-panel .captions-row');
    await expect(rows).toHaveCount(4);
    await expect(page.locator('.captions-panel .captions-row.picked')).toContainText('Captions make every word heard.');

    await settle(page);
    await page.screenshot({ path: path.join(SHOTS, `editor-layers-${scheme}.png`) });
    await page.locator('#tl-mode [data-mode="tracks"]').click();
    await settle(page);
    await page.screenshot({ path: path.join(SHOTS, `editor-tracks-${scheme}.png`) });
    await page.locator('#panel-timeline').screenshot({ path: path.join(SHOTS, `timeline-tracks-${scheme}.png`) });
    await page.locator('#tl-mode [data-mode="layers"]').click();
    await settle(page);
    await page.locator('#panel-timeline').screenshot({ path: path.join(SHOTS, `timeline-layers-${scheme}.png`) });

    // Export dialog: captions choices appear in the Settings-sheet anatomy.
    await page.evaluate(() => (window as any).PM.cmd('export'));
    await page.locator('.export-destination', { hasText: 'Video' }).click();
    await expect(page.locator('.settings-row', { hasText: 'Burn in captions' })).toBeVisible();
    await expect(page.locator('.settings-row', { hasText: 'Caption file' })).toBeVisible();
    await page.locator('.settings-row', { hasText: 'Caption file' }).scrollIntoViewIfNeeded();
    await settle(page);
    await page.locator('.export-modal').screenshot({ path: path.join(SHOTS, `export-dialog-${scheme}.png`) });
    await page.keyboard.press('Escape');

    await page.evaluate(() => (window as any).PM.cmd('exportCaptions'));
    await expect(page.locator('.captions-export .settings-row', { hasText: 'Format' })).toBeVisible();
    await settle(page);
    await page.locator('.export-modal').screenshot({ path: path.join(SHOTS, `export-captions-${scheme}.png`) });
    await page.keyboard.press('Escape');
  });
}

test('every preset renders a caption', async ({ session }) => {
  const { page } = session;
  await mkdir(SHOTS, { recursive: true });
  const id = await seed(page);
  for (const preset of ['classic', 'boxed', 'whisper', 'spotlight', 'pop', 'paper']) {
    const image = await page.evaluate(async ({ id, preset }) => {
      const PM = (window as any).PM;
      PM.Edit.apply({ type: 'edit_captions', target: id, op: 'style', style: { preset } });
      return PM.Export.snapshotAsync ? PM.Export.snapshotAsync(3.3, 960) : PM.Export.snapshot(3.3, 960);
    }, { id, preset });
    expect(String(image)).toMatch(/^data:image\//);
    await writeFile(path.join(SHOTS, `preset-${preset}.png`), Buffer.from(String(image).split(',')[1]!, 'base64'));
  }
});
