import { test, expect } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Error checks', w: 640, h: 360, dur: 5, fps: 30 }) }));
    PM.ProjectsScreen.hide();
  });
});

// These tests own a disposable profile. Flush it explicitly, then exit without
// coupling error-card checks to the native close-confirmation workflow.
test.afterEach(async ({ session }) => {
  await session.page.evaluate(async () => { await (window as any).PM.flushProject(); });
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});

test('agent errors and app failures remain readable, inspectable and dismissible', async ({ session }) => {
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).waitFor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.theme.apply('dark');
    Object.assign(PM.AgentUI.state, {
      phase: 'result', legacyPhase: 'conversation', activity: '', trace: [], panelRun: null,
      conversation: [
        { role: 'user', text: 'Continue working on the animation' },
        { role: 'assistant', error: true, entering: true, text: 'The agent failed: Error: thread/resume: thread/resume failed: thread 01a095e4-b0d6-7732-a641-5671c55bc13c already has an active writer (code -32600)' }
      ], run: null
    });
  });
  const panel = page.locator('#panel-agent');
  await panel.evaluate(el => (el as HTMLElement).style.setProperty('--set-panel-height', '700px'));
  for (const width of [240, 320, 480]) {
    await panel.evaluate((el, width) => { (el.closest('.dock') as HTMLElement).style.flex = `0 0 ${width}px`; }, width);
    const notice = panel.locator('.error-notice');
    await expect(notice.locator('strong')).toHaveText('Agent session couldn’t reopen');
    await expect(notice.locator('pre')).not.toBeVisible();
    await panel.screenshot({ path: `/tmp/powermove-error-${width}.png` });
    await notice.getByText('Technical details', { exact: true }).click();
    await expect(notice.locator('pre')).toContainText('-32600');
    const overflow = await notice.evaluate(el => el.scrollWidth > el.clientWidth + 1);
    expect(overflow).toBe(false);
    await notice.getByText('Technical details', { exact: true }).click();
  }
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.toast('Could not save project: ENOSPC: no space left on device');
    PM.toast('Could not import missing.mov');
    PM.toast('Could not import missing.mov');
    PM.toast('Saved workspace');
  });
  await expect(page.locator('.toast[data-toast-error]')).toHaveCount(2);
  await expect(page.locator('.toast').getByText('Your disk is full', { exact: true })).toBeVisible();
  await page.screenshot({ path: '/tmp/powermove-error-notifications.png' });
  await page.getByRole('button', { name: 'Dismiss notification' }).first().click();
  await expect(page.locator('.toast[data-toast-error]')).toHaveCount(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('failed asynchronous dialog actions keep the input and allow another attempt', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const input = document.createElement('input');
    input.setAttribute('aria-label', 'Project name');
    input.value = 'My animation';
    let attempts = 0;
    PM.modal({ title: 'Save project', body: input, actions: [{ label: 'Save', run: async () => {
      await new Promise(resolve => setTimeout(resolve, 80));
      if (attempts++ === 0) throw new Error('ENOSPC: no space left on device');
    } }] });
  });
  await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('dialog').getByText('Your disk is full', { exact: true })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Project name' })).toHaveValue('My animation');
  await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('sending a message leaves the saved Takes archive untouched and keeps Undo available', async ({ session }) => {
  const { page } = session;
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).waitFor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.AgentThreadTitles.generate = async () => JSON.stringify({ title: 'Send check' });
    const get = PM.store.get.bind(PM.store), set = PM.store.set.bind(PM.store);
    (window as any).__takeReads = 0;
    (window as any).__takeWrites = 0;
    PM.store.get = (key: string, fallback: unknown) => {
      if (key === 'takes') (window as any).__takeReads++;
      return get(key, fallback);
    };
    PM.store.set = (key: string, value: unknown) => {
      if (key === 'takes') (window as any).__takeWrites++;
      return set(key, value);
    };
    PM.CodexBridge.request = async () => ({ text: JSON.stringify({ summary: 'Finished', commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: [] });
    PM.AgentUI.submit('Continue');
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase)).toBe('result');
  expect(await page.evaluate(() => ({ reads: (window as any).__takeReads, writes: (window as any).__takeWrites }))).toEqual({ reads: 0, writes: 0 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
