import { expect, test } from './helpers/app';
import { _electron } from 'playwright';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { repoRoot } from './helpers/app';



test('custom selection commits before returning and keyboard palette has no travel', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const select = document.createElement('select');
    select.id = 'motion-select'; select.setAttribute('aria-label', 'Motion fixture');
    select.innerHTML = '<option value="one">One</option><option value="two">Two</option>';
    Object.assign(select.style, { position: 'fixed', left: '40px', top: '80px', zIndex: '999' });
    document.body.append(select);
  });
  for (let n = 0; n < 4; n++) {
    await page.getByRole('combobox', { name: 'Motion fixture', exact: true }).click();
    const outcome = await page.locator('.pm-menu[data-state="open"][role="listbox"] .pm-menu-item').nth(n % 2).evaluate(row => {
      (row as HTMLElement).click();
      return { value: (document.getElementById('motion-select') as HTMLSelectElement).value, closing: row.parentElement?.dataset.state };
    });
    expect(outcome).toEqual({ value: n % 2 ? 'two' : 'one', closing: 'closed' });
    await page.keyboard.press('Escape');
  }
  await page.evaluate(() => document.getElementById('motion-select')?.remove());
  await page.keyboard.press('Meta+k');
  const palette = page.locator('#palette');
  await expect(palette).toBeVisible();
  expect(await palette.evaluate(el => getComputedStyle(el).animationName)).toBe('none');
  const input = palette.getByRole('combobox');
  await input.press('ArrowDown'); await input.press('ArrowDown'); await input.press('ArrowUp');
  const state = await palette.evaluate(el => {
    const glider = el.querySelector<HTMLElement>('.pglider')!;
    const row = el.querySelector<HTMLElement>('[aria-selected="true"]')!;
    return { transform: glider.style.transform, top: row.offsetTop, properties: getComputedStyle(glider).transitionProperty,
      active: el.querySelector('input')?.getAttribute('aria-activedescendant'), id: row.id };
  });
  expect(state.transform).toBe(`translateY(${state.top}px)`);
  expect(state.properties).toBe('opacity'); expect(state.active).toBe(state.id);
  await input.press('Escape'); await expect(palette).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('library composer is ready during the flight and rapid Back/reopen preserves its panel', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.evaluate(() => { const PM = (window as any).PM; PM.LibraryUI.reveal('notes'); PM.LibraryUI.open(); });
  const library = page.locator('#library-screen');
  await expect(library.locator('[data-panel-id="notes"] .library-clone')).toBeAttached();
  const timing = await library.getByRole('button', { name: 'Edit Notes', exact: true }).evaluate(button => {
    (button as HTMLElement).click();
    return { composer: !!document.querySelector('.library-chat .agent-composer [role="textbox"]'),
      snapshot: !!document.querySelector('.library-stage-ghost'),
      flying: !!document.querySelector('.library-stage-frame')?.getAnimations().length };
  });
  expect(timing).toEqual({ composer: true, snapshot: true, flying: true });
  await library.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Make the notes panel more compact');
  await page.evaluate(() => {
    const edit = () => document.querySelector<HTMLButtonElement>('[data-panel-id="notes"] .library-card-hit')!.click();
    document.querySelector<HTMLElement>('[aria-label="Back to panels"]')!.click(); edit();
    document.querySelector<HTMLElement>('[aria-label="Back to panels"]')!.click(); edit();
  });
  await expect(library.locator('.library-stage-live #panel-notes')).toBeVisible();
  await expect(library.locator('.library-fly')).toHaveCount(0);
  await library.press('Escape'); await expect(library.locator('.library-shell')).toBeVisible();
  await library.press('Escape'); await expect(library).toBeHidden();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('reduced motion disables chrome travel while preserving the authored viewport', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => (window as any).PM.LibraryUI.open());
  const library = page.locator('#library-screen');
  await library.getByRole('button', { name: 'Edit Notes', exact: true }).click();
  await expect(library.locator('.library-stage-live #panel-notes')).toBeVisible();
  expect(await page.locator('.library-stage-frame').evaluate(el => el.getAnimations().length)).toBe(0);
  expect(await library.locator('.pm-back-title-chev').evaluate(el => getComputedStyle(el).transitionProperty)).toBe('none');
  await library.press('Escape'); await library.press('Escape');
  await page.keyboard.press('Meta+k');
  expect(await page.locator('#palette').evaluate(el => el.getAnimations({ subtree: true }).flatMap(animation => (animation.effect as KeyframeEffect).getKeyframes()).filter(frame => 'transform' in frame || 'translate' in frame || 'scale' in frame))).toEqual([]);
  await page.keyboard.press('Escape');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const mode of ['skip', 'escape', 'reduce'] as const) {
  test(`onboarding ${mode} action reaches setup without waiting for the intro`, async () => {
    const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-motion-onboard-'));
    const env = { ...process.env as Record<string, string>, POWERMOVE_USER_DATA: userData,
      POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_TEST_ONBOARDING: '1', POWERMOVE_DEVTOOLS: '0',
      POWERMOVE_DISABLE_RUNTIME_UPDATES: '1',
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'), POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
    delete env.ELECTRON_RUN_AS_NODE;
    const app = await _electron.launch({ args: ['.'], cwd: repoRoot, env });
    try {
      {
        const intro = await app.firstWindow();
        await intro.waitForLoadState('domcontentloaded');
        await expect(intro.getByRole('button', { name: 'Skip intro' })).toBeVisible();
        if (mode === 'reduce') await intro.emulateMedia({ reducedMotion: 'reduce' });
        else if (mode === 'skip') await intro.getByRole('button', { name: 'Skip intro' }).click();
        else await intro.keyboard.press('Escape');
      }
      await expect.poll(() => app.windows().some(p => p.url().includes('welcome.html'))).toBe(true);
      const welcome = app.windows().find(p => p.url().includes('welcome.html'))!;
      await expect(welcome.locator('#begin')).toBeVisible();
      expect(app.windows().filter(p => p.url().includes('/onboarding/animation.html'))).toHaveLength(0);
      await welcome.locator('#begin').click();
      await expect(welcome.locator('#timeline-title')).toBeVisible();
      if (mode === 'reduce') { await welcome.emulateMedia({ reducedMotion: 'reduce' }); expect(await welcome.locator('.timeline-step > h1').evaluate(el => getComputedStyle(el).animationName)).toBe('none'); }
    } finally { await app.close(); await rm(userData, { recursive: true, force: true }); }
  });
}

test('tab motion stops on preference changes and reduced-motion reorders remain immediate', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const ids = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const ids = [PM.proj.id];
    for (const name of ['Motion second', 'Motion third']) {
      const project = PM.mkProject({ name }); PM.Projects.put(project); await PM.Tabs.activate(project.id); ids.push(project.id);
    }
    return ids;
  });
  await page.evaluate(id => (window as any).PM.Tabs.move(id, 0), ids[2]);
  await expect.poll(() => page.locator('#tabs .project-doc').evaluateAll(nodes => nodes[0]?.getAttribute('data-tab-id'))).toBe(ids[2]);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect.poll(() => page.locator('#tabs').evaluate(el => el.getAnimations({ subtree: true }).filter(animation =>
    (animation.effect as KeyframeEffect).getKeyframes().some(frame => 'transform' in frame)).length)).toBe(0);
  await page.evaluate(id => (window as any).PM.Tabs.move(id, 0), ids[1]);
  await expect.poll(() => page.locator('#tabs .project-doc').evaluateAll(nodes => nodes[0]?.getAttribute('data-tab-id'))).toBe(ids[1]);
  expect(await page.locator('#tabs').evaluate(el => el.getAnimations({ subtree: true }).filter(animation =>
    (animation.effect as KeyframeEffect).getKeyframes().some(frame => 'transform' in frame)).length)).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
