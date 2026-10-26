import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, launchApp, test } from './helpers/app';

test('restart opens only the previously active project tab', async ({ session }) => {
  await session.openEditor();
  const ids = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const ids: string[] = [];
    for (const name of ['First project', 'Second project', 'Third project']) {
      const project = PM.mkProject({ name });
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      ids.push(project.id);
    }
    await PM.Tabs.activate(ids[0]);
    await PM.flushProject();
    return ids;
  });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Tabs.list().length)).toBe(4);
  await session.relaunch();
  await session.page.waitForFunction(id => (window as any).PM.proj.id === id, ids[0]);
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Tabs.list().length)).toBe(1);
  await expect(session.page.locator('.tb-title-name')).toHaveText('First project');
  expect(await session.page.evaluate(ids => ids.every(id => !!(window as any).PM.Projects.get(id)), ids)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('restart restores one project from the last focused window', async ({ session }) => {
  await session.openEditor();
  const active = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const first = PM.mkProject({ name: 'Focused project' });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: first }));
    const other = PM.mkProject({ name: 'Other window project' });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: other }));
    await PM.openProjectInNewWindow(other.id);
    await PM.openProjectHere(first.id);
    await PM.flushProject();
    return first.id;
  });
  const window = await session.app.browserWindow(session.page);
  await window.evaluate(window => window.emit('focus'));
  await session.relaunch();
  expect(session.app.windows()).toHaveLength(1);
  expect(await session.page.evaluate(() => (window as any).PM.proj.id)).toBe(active);
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Tabs.list().length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('turning off project restoration starts on Projects without opening another recent project', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.store.set('restoreWindows', false);
    await PM.flushProject();
  });
  await session.relaunch();
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Tabs.list().length)).toBe(0);
  await expect(session.page.locator('.tb-title-btn')).toHaveCount(0);
  expect(await session.page.evaluate(() => (window as any).PM.ProjectsScreen.isOpen)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('an old single-autosave profile restores its only project', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-legacy-startup-'));
  await mkdir(path.join(directory, 'store'));
  await writeFile(path.join(directory, 'store', 'autosave.json'), JSON.stringify(JSON.stringify({ proj: {
    id: 'legacy-project', name: 'Legacy recovery', w: 640, h: 360, fps: 30, dur: 4, layers: [], assets: {}, comps: {},
  } })));
  const session = await launchApp({ userData: directory });
  try {
    expect(await session.page.evaluate(() => (window as any).PM.proj.id)).toBe('legacy-project');
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.Tabs.list().length)).toBe(1);
    await expect(session.page.locator('.tb-title-name')).toHaveText('Legacy recovery');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); await rm(directory, { recursive: true, force: true }); }
});
