import { expect, test } from './helpers/app';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

test('identifies a slow extension and lets the user turn it off', async ({ session }) => {
  const directory = path.join(session.userData, 'extensions', 'slow-preview-test');
  await mkdir(directory, { recursive: true });
  await writeFile(path.join(directory, 'manifest.json'), JSON.stringify({
    id: 'slow-preview-test', name: 'Slow preview test', version: '1.0.0', apiVersion: 1,
    entry: 'index.js', contributes: ['commands'],
  }));
  await writeFile(path.join(directory, 'index.js'), `export default function(api) {
    api.commands.register({ id: 'slow-preview-test.measure', label: 'Measure slow callback', run() {
      const end = performance.now() + 65; while (performance.now() < end) {}
    } });
  }`);
  await session.relaunch();
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Performance warning' }) }));
    PM.ProjectsScreen.hide();
  });
  await session.page.waitForFunction(() => {
    const command = (window as any).PM.Kernel.commands.get('slow-preview-test.measure');
    if (!command) return false;
    command.run();
    return true;
  });
  const warning = session.page.getByRole('button', { name: /Performance · Slow preview test/ });
  await expect(warning).toBeVisible(); await warning.click();
  const dialog = session.page.getByRole('dialog', { name: 'What is slowing the editor down', exact: true });
  await expect(dialog).toContainText('on the editor thread');
  await dialog.getByRole('button', { name: 'Turn off', exact: true }).click();
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Kernel.api('e2e-check')
    .extensions.list().find((entry: any) => entry.id === 'slow-preview-test')?.enabled)).toBe(false);
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Kernel.commands.has('slow-preview-test.measure'))).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
