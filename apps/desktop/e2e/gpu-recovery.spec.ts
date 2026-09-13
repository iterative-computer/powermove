import { expect, test } from './helpers/app';

test('restores the viewer after a GPU context reset without changing the project', async ({ session }) => {
  await session.page.waitForFunction(() => !!(window as any).PM?.GL?.gl);
  const before = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'GPU recovery', bg: '#ff0000' }) }));
    PM.ProjectsScreen.hide();
    (window as any).__loseContext = PM.GL.gl.getExtension('WEBGL_lose_context');
    if (!(window as any).__loseContext) return null;
    return JSON.stringify(PM.proj);
  });
  test.skip(before === null, 'GPU reset extension unavailable');
  await session.page.evaluate(() => (window as any).__loseContext.loseContext());
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.GL.contextLost)).toBe(true);
  await session.page.evaluate(() => (window as any).__loseContext.restoreContext());
  await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.GL.gl && !(window as any).PM.GL.contextLost)).toBe(true);
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.GL.progs.size)).toBeGreaterThan(0);
  expect(await session.page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
