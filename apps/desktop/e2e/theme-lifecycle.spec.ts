import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('theme appearance fully un-applies across workspaces, toggles, reload and restart', async ({ session }) => {
  test.setTimeout(60_000);
  const id = 'test-reversible-theme';
  const dir = path.join(session.userData, 'extensions', id);
  await mkdir(dir, { recursive: true });
  await writeFile(path.join(dir, 'manifest.json'), JSON.stringify({ id, name: 'Reversible theme', version: '1.0.0', apiVersion: 3,
    contributes: ['themes', 'commands'], permissions: ['full-access'] }));
  await writeFile(path.join(dir, 'index.ts'), `import type { PowermoveAPI } from 'powermove';
export default function activate(api: PowermoveAPI) {
  api.theme.register({ id: '${id}.theme', name: 'Test theme', scheme: 'dark',
    tokens: { '--test-theme-token': '#abcdef' }, rootAttributes: { 'data-test-theme': 'on' },
    css: ':root { --test-theme-css: active; }' });
  function apply() {
    if (!api.workspace.current()) return;
    api.storage.set('applied', true);
    api.theme.activate('${id}.theme');
    api.theme.setScheme('dark');
    api.workspace.mutate(ws => {
      ws.density = 'compact'; ws.chrome.previewCornerRadius = 'rounded';
      ws.chrome.timeline.rowHeight = 22; ws.chrome.timeline.gutterWidth = 340;
      for (const dock of ws.layout.docks) for (const panel of dock.panels) {
        if (panel.id === 'timeline') panel.size = 280;
      }
    });
  }
  api.commands.register({ id: '${id}.apply', label: 'Apply test theme', run: apply });
  if (api.storage.get('applied')) apply();
  // Deliberately no manual cleanup: the host owns these changes.
}`);
  await session.relaunch();
  await session.openEditor();
  let { page } = session;
  await page.waitForFunction(() => (window as any).PM.Kernel.commands.has('test-reversible-theme.apply'));
  const initial = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const scheme = PM.Kernel.theme.scheme;
    const first = PM.WS.create({ base: PM.WS.current.id, name: 'First theme workspace' }).id;
    const second = PM.WS.create({ base: first, name: 'Second theme workspace' }).id;
    const before = [first, second].map(id => {
      const ws = PM.WS.get(id);
      return { density: ws.density, corners: ws.chrome.previewCornerRadius, row: ws.chrome.timeline.rowHeight,
        gutter: ws.chrome.timeline.gutterWidth, sizes: ws.layout.docks.flatMap((d: any) => d.panels.filter((p: any) => p.id === 'timeline').map((p: any) => p.size)) };
    });
    await PM.Kernel.api('test').commands.run('test-reversible-theme.apply');
    PM.WS.activate(first);
    await PM.Kernel.api('test').commands.run('test-reversible-theme.apply');
    // Retain a subsequent user edit in the first workspace.
    PM.WS.mutate((ws: any) => { ws.chrome.timeline.gutterWidth = 280; }, { inPlace: true });
    return { ids: [first, second], before, scheme };
  });
  await expect(page.locator('html')).toHaveAttribute('data-test-theme', 'on');
  const turnOff = () => page.evaluate(async id => { await (window as any).PM.Kernel.api('test').extensions.setEnabled(id, false); }, id);
  const assertRestored = async () => {
    await expect(page.locator('html')).not.toHaveAttribute('data-test-theme', 'on');
    await page.waitForFunction(({ ids, before }) => {
      const PM = (window as any).PM;
      return ids.every((id, index) => PM.WS.get(id)?.chrome?.timeline?.rowHeight === before[index].row);
    }, initial);
    expect(await page.evaluate(ids => {
      const PM = (window as any).PM;
      return ids.map(id => {
        const ws = PM.WS.get(id);
        return { density: ws.density, corners: ws.chrome.previewCornerRadius, row: ws.chrome.timeline.rowHeight,
          gutter: ws.chrome.timeline.gutterWidth, sizes: ws.layout.docks.flatMap((d: any) => d.panels.filter((p: any) => p.id === 'timeline').map((p: any) => p.size)) };
      });
    }, initial.ids)).toEqual(initial.before.map((value, i) => ({ ...value, gutter: i === 0 ? 280 : value.gutter })));
    expect(await page.evaluate(() => ({ token: document.documentElement.style.getPropertyValue('--test-theme-token'),
      css: getComputedStyle(document.documentElement).getPropertyValue('--test-theme-css').trim() }))).toEqual({ token: '', css: '' });
    expect(await page.evaluate(() => (window as any).PM.Kernel.theme.scheme)).toBe(initial.scheme);
  };
  await turnOff(); await assertRestored();
  await page.evaluate(async id => { await (window as any).PM.Kernel.api('test').extensions.setEnabled(id, true); }, id);
  await expect(page.locator('html')).toHaveAttribute('data-test-theme', 'on');
  await page.evaluate(async id => {
    const PM = (window as any).PM;
    await PM.Kernel.loader.reload(id);
  }, id);
  await expect(page.locator('html')).toHaveAttribute('data-test-theme', 'on');
  await session.relaunch(); page = session.page; await session.openEditor();
  await expect(page.locator('html')).toHaveAttribute('data-test-theme', 'on');
  await turnOff(); await assertRestored();
  await page.evaluate(async id => { await (window as any).PM.Kernel.api('test').extensions.setEnabled(id, true); }, id);
  await expect(page.locator('html')).toHaveAttribute('data-test-theme', 'on');
  await page.evaluate(async id => { await (window as any).PM.Kernel.api('test').extensions.remove(id); }, id);
  await assertRestored();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
