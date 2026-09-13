import { expect, test } from './helpers/app';

test('identifies a slow extension and lets the user turn it off', async ({ session }) => {
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Performance warning' }) }));
    PM.ProjectsScreen.hide();
    await (window as any).powermove.extensions.create({
      manifest: { id: 'slow-preview-test', name: 'Slow preview test', version: '1.0.0', apiVersion: 1, entry: 'index.js', contributes: ['commands'] },
      files: { 'index.js': `export default function(api) { api.commands.register({id:'slow-preview-test.measure',label:'Measure slow callback',run(){const end=performance.now()+65;while(performance.now()<end){}}}); }` }
    });
  });
  await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.Kernel.commands.get('slow-preview-test.measure'))).toBe(true);
  await session.page.evaluate(() => (window as any).PM.Kernel.commands.get('slow-preview-test.measure').run());
  const warning = session.page.getByRole('button', { name: /Performance · Slow preview test/ });
  await expect(warning).toBeVisible(); await warning.click();
  const dialog = session.page.getByRole('dialog', { name: 'What is slowing the editor down', exact: true });
  await expect(dialog).toContainText('on the editor thread');
  await dialog.getByRole('button', { name: 'Turn off', exact: true }).click();
  await expect.poll(() => session.page.evaluate(async () => (await (window as any).powermove.extensions.list()).find((entry: any) => entry.id === 'slow-preview-test')?.enabled)).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
