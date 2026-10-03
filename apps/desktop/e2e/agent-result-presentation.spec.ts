import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { test, expect } from './helpers/app';

test('agent results preserve work above the reply and omit file and external activity panels', async ({ session }, testInfo) => {
  await session.openEditor(); await session.openAgent();
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  const { page } = session;
  const output = process.env.POWERMOVE_AGENT_ARTIFACTS || testInfo.outputPath('visuals');
  await mkdir(output, { recursive: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).waitFor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.theme.apply('dark');
    // This is a visual state fixture; late bridge publications must not replace it.
    PM.AgentUI.update({ flush: true });
    PM.AgentUI.update = () => {};

    PM.Kernel.loader.records = () => [{ id: 'pexels-browser', manifest: { name: 'Pexels Browser' }, health: { state: 'ok' } }];
    const entries = PM.Kernel.panels.entries.bind(PM.Kernel.panels);
    PM.Kernel.panels.entries = () => [...entries(), { id: 'pexels-panel', ownerId: 'pexels-browser', item: { id: 'pexels-panel', title: 'Pexels Browser' } }];
    (window as any).__openedResultPanel = '';
    PM.LibraryUI.reveal = (id: string) => { (window as any).__openedResultPanel = id; };
    const steps = Array.from({ length: 18 }, (_, index) => ({
      kind: 'tool', id: `step-${index}`, toolName: ['bash', 'web', 'edit_file'][index % 3],
      label: `Verified project operation ${index + 1}`, status: index === 17 ? 'error' : 'done'
    }));
    Object.assign(PM.AgentUI.state, {
      phase: 'result', legacyPhase: 'result', activity: '', trace: [], panelRun: null,
      conversation: [
        { role: 'trace', durationMs: 543000, steps: [
          { kind: 'thought', id: 'thinking', label: 'Check the media before importing.', live: false }, ...steps
        ] },
        { role: 'assistant', text: 'Created a Pexels panel with photo/video search, thumbnails, and import into Assets.' },
        { role: 'assistant', text: 'Added mod Pexels Browser', modResult: {
          id: 'pexels-browser', name: 'Pexels Browser', action: 'created', status: 'ready'
        } }
      ],
      run: { autonomous: true, changed: true, externalActions: ['Downloaded reference media'], artifacts: ['check.cjs', 'library.cjs', 'panel.cjs', 'verification.txt'].map((name, index) => ({
        name, path: `artifacts/${name}`, mime: index === 3 ? 'text/plain' : 'application/octet-stream', size: [4096, 3072, 12288, 1024][index]
      })) }
    });
  });
  const panel = page.locator('#panel-agent');
  await panel.evaluate(el => (el.closest('#agent-popover') as HTMLElement).style.height = '700px');
  for (const width of [240, 320, 480]) {
    await panel.evaluate((el, width) => { (el.closest('#agent-popover') as HTMLElement).style.width = `${width}px`; }, width);
    await page.locator('.agent-scroll').evaluate(el => { el.scrollTop = 0; });
    await panel.screenshot({ path: path.join(output, `result-layout-${width}.png`) });
    await expect(page.locator('.agent-work-log')).toHaveCount(0);
    await expect(page.locator('.agent-thought-prose')).toBeVisible();
    await expect(page.locator('.agent-thought-prose')).toContainText('Check the media before importing.');
    // The header reads "18 tool calls · 1 failed · …" inline; the status must
    // stay one line tall and inside the header box at every width.
    const layout = await page.locator('.agent-tool-activity summary').evaluate(el => {
      const box = el.getBoundingClientRect();
      const status = el.querySelector('em')!.getBoundingClientRect();
      return { inside: status.left >= box.left && status.right <= box.right + 1, statusLines: status.height / parseFloat(getComputedStyle(el.querySelector('em')!).lineHeight) };
    });
    expect.soft(layout.inside).toBe(true);
    expect.soft(layout.statusLines).toBeLessThan(1.5);
    const composer = await page.locator('.agent-composer').evaluate(el => {
      const attach = el.querySelector('.agent-attach')!.getBoundingClientRect();
      const send = el.querySelector('.agent-send')!.getBoundingClientRect();
      const input = el.querySelector('[role="textbox"]')!.getBoundingClientRect();
      return { border: getComputedStyle(el).borderTopWidth, attach: attach.y + attach.height / 2, send: send.y + send.height / 2, inputBottom: input.bottom, attachTop: attach.top };
    });
    expect.soft(composer.border).toBe('0px');
    // Controls share the foot row; the draft sits alone above them.
    expect.soft(Math.abs(composer.attach - composer.send)).toBeLessThanOrEqual(1);
    expect.soft(composer.inputBottom).toBeLessThanOrEqual(composer.attachTop + 1);
    const overflow = await page.locator('.agent-shell').evaluate(root => [root, ...root.querySelectorAll('*')].filter(el =>
      el.clientWidth > 0 && !el.classList.contains('panel-sr-only') && getComputedStyle(el).display !== 'inline'
      && getComputedStyle(el).textOverflow !== 'ellipsis' && el.scrollWidth > el.clientWidth + 1).map(el => ({
        tag: el.tagName, class: el.getAttribute('class'), width: el.clientWidth, scroll: el.scrollWidth, text: el.textContent?.slice(0, 80)
      })));
    expect.soft(overflow).toEqual([]);
  }
  await expect(page.locator('.agent-mod-result')).toContainText('Pexels Browser');
  await page.getByRole('button', { name: 'Open Pexels Browser', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__openedResultPanel)).toBe('pexels-panel');
  await expect(page.locator('.agent-artifacts')).toHaveCount(0);
  await expect(page.locator('.agent-external-actions')).toHaveCount(0);
  await page.locator('.agent-tool-activity summary').click();
  await expect(page.locator('.agent-tool-details')).toBeVisible();
  await expect(page.locator('.agent-tool-details > div')).toHaveCount(18);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a completed response stays visible when the user sends a follow-up', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentUI.setAccess('project');
    (window as any).__responseRuns = [];
    PM.CodexBridge.request = (_prompt: unknown, _schema: unknown, _images: unknown, options: any) => new Promise(resolve => {
      (window as any).__responseRuns.push({ options, resolve });
    });
  });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Inspect the composition');
  await composer.press('Enter'); await session.openAgent();
  await expect.poll(() => page.evaluate(() => (window as any).__responseRuns.length)).toBe(1);
  await page.evaluate(() => {
    const run = (window as any).__responseRuns[0];
    run.options.onTrace({ kind: 'answer', text: 'The clip is placed full-frame, 0–22 s, centered.' });
    run.options.onTrace({ kind: 'thought', text: 'Checking the live composition.' });
    run.options.onTrace({ kind: 'answer', text: 'The composition is ready for review.' });
    run.resolve({ text: JSON.stringify({ summary: 'The composition is ready for review.', commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: [] });
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
  const response = page.locator('.agent-trace.is-archived');
  await expect(response.getByText('The clip is placed full-frame, 0–22 s, centered.', { exact: true })).toBeVisible();
  await expect(response.getByText('The composition is ready for review.', { exact: true })).toBeVisible();
  const before = await response.innerText();
  await composer.fill('Make it as editable layers');
  await composer.press('Enter'); await session.openAgent();
  await expect.poll(() => page.evaluate(() => (window as any).__responseRuns.length)).toBe(2);
  await expect(response.getByText('The clip is placed full-frame, 0–22 s, centered.', { exact: true })).toBeVisible();
  await expect(response.getByText('The composition is ready for review.', { exact: true })).toBeVisible();
  expect(await response.innerText()).toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
