import type { Page } from '@playwright/test';

const RESOLUTION_LABELS: Record<string, string> = { auto: 'Automatic', '1': 'Full', '0.5': 'Half', '0.25': 'Quarter' };

/** Chooses a preview resolution from the viewer's Preview quality menu (the
 * popover that replaced the resolution `<select>`, plan appendix D "Viewer
 * menu"). `value` uses the old select's values: auto, 1, 0.5, 0.25. */
export async function choosePreviewResolution(page: Page, value: string): Promise<void> {
  const label = RESOLUTION_LABELS[value];
  if (!label) throw new Error(`Unknown preview resolution ${value}`);
  await page.getByRole('button', { name: 'Preview quality', exact: true }).click();
  await page.getByRole('group', { name: 'Resolution', exact: true })
    .getByRole('menuitemradio', { name: label, exact: true }).click();
}
