import path from 'node:path';

import type { Page } from 'playwright';

import { repoRoot } from './app';

export function fixturePath(name: string): string {
  return path.join(repoRoot, 'e2e/fixtures', name);
}

export async function importFixture(page: Page, name: string): Promise<void> {
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(fixturePath(name));
}
