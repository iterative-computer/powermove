import { test, expect } from './helpers/app';

test('Command+A selects the full agent composer draft', async ({ session }) => {
  const page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Select this entire composer draft');
  await composer.press('Meta+A');
  await expect.poll(() => composer.evaluate((textarea: HTMLTextAreaElement) => ({
    start: textarea.selectionStart,
    end: textarea.selectionEnd,
    length: textarea.value.length,
  }))).toEqual({ start: 0, end: 33, length: 33 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('selected agent text copies and pastes normally without copying layers', async ({ session }) => {
  const page = session.page;
  const reply = 'Selection-ready agent reply';
  await session.app.evaluate(({ clipboard }) => clipboard.writeText(''));
  await page.evaluate((text) => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async () => ({
      text: JSON.stringify({ summary: text, commands: [], artifacts: [], externalActions: [], notes: [] })
    });
  }, reply);

  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Give me selectable text');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const answer = page.locator('.agent-msg.assistant p').filter({ hasText: reply });
  await expect(answer).toBeVisible();
  await answer.evaluate((element) => {
    (document.activeElement as HTMLElement | null)?.blur?.();
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(element);
    selection?.removeAllRanges();
    selection?.addRange(range);
  });

  const selectedLayers = await page.evaluate(() => [...(window as any).PM.sel.layers]);
  await page.keyboard.press('Meta+C');
  await expect.poll(() => session.app.evaluate(({ clipboard }) => clipboard.readText())).toBe(reply);
  expect(await page.evaluate(() => [...(window as any).PM.sel.layers])).toEqual(selectedLayers);

  await composer.click();
  await page.keyboard.press('Meta+V');
  await expect(composer).toHaveValue(reply);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('steering stays in the active run and preserves the transcript before it', async ({ session }, testInfo) => {
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    (window as any).__steeringProof = { requests: 0, steering: [] as string[] };
    PM.CodexBridge.request = async (_prompt: string, _schema: unknown, _images: unknown[], options: any) => {
      (window as any).__steeringProof.requests += 1;
      options.onTrace({ kind: 'answer', text: 'I have started building the blur.' });
      return await new Promise(() => {});
    };
    PM.CodexBridge.steer = async (prompt: string) => {
      (window as any).__steeringProof.steering.push(prompt);
      return true;
    };
  });

  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Make a progressive blur effect');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop current run', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__steeringProof.requests)).toBe(1);
  await expect(page.locator('.agent-trace-text')).toHaveText('I have started building the blur.');

  await composer.fill('continue');
  await composer.press('Enter');
  await expect.poll(() => page.evaluate(() => (window as any).__steeringProof.steering.length)).toBe(1);
  const userTurns = page.locator('.agent-msg.user');
  await expect(userTurns).toHaveCount(2);
  await expect(userTurns.nth(1)).toHaveClass(/is-steering/);
  await expect(userTurns.nth(1).locator('.agent-bubble')).toHaveText('continue');
  const transcriptOrder = await page.locator('.agent-scroll > :is(.agent-msg, .agent-trace.is-archived)').evaluateAll((items) =>
    items.map((item) => item.textContent?.trim()).filter(Boolean));
  expect(transcriptOrder).toEqual([
    'Make a progressive blur effect',
    'I have started building the blur.',
    'continue'
  ]);
  const gap = await page.evaluate(() => {
    const trace = document.querySelector('.agent-trace.is-archived')?.getBoundingClientRect();
    const steer = document.querySelector('.agent-msg.user.is-steering')?.getBoundingClientRect();
    return trace && steer ? steer.top - trace.bottom : Number.POSITIVE_INFINITY;
  });
  expect(gap).toBeLessThanOrEqual(10);
  expect(await page.evaluate(() => (window as any).__steeringProof.requests)).toBe(1);
  await testInfo.attach('hidden-agent-steering', { body: await page.screenshot(), contentType: 'image/png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('hidden renderer switches threads, keeps drafts, and restores history after relaunch', async ({ session }) => {
  const page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  const picker = page.getByRole('button', { name: 'Switch thread', exact: true });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  const activeThread = () => page.evaluate(() => (window as any).PM.AgentUI.state.threadId as string);
  const pick = async (id: string) => {
    await picker.click();
    await page.locator(`.thread-row`).nth(await page.evaluate((threadId) => (window as any).PM.AgentUI.state.threads
      .findIndex((thread: any) => thread.id === threadId), id)).click();
  };
  await expect(picker).toBeVisible();
  const first = await activeThread();
  await composer.fill('First thread draft');
  await page.getByRole('button', { name: 'New thread', exact: true }).click();
  const second = await activeThread();
  expect(second).not.toBe(first);
  await expect(composer).toHaveValue('');
  await composer.fill('Second thread draft');
  await pick(first);
  await expect(composer).toHaveValue('First thread draft');
  await pick(second);
  await expect(composer).toHaveValue('Second thread draft');
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.store.get(`agentThreads.${PM.proj.id}`)?.activeId;
  })).toBe(second);
  await session.relaunch();
  await expect.poll(() => session.page.evaluate(() => {
    const state = (window as any).PM.AgentUI.state;
    return { activeId: state.threadId, threadIds: state.threads.map((thread: any) => thread.id) };
  })).toEqual({ activeId: second, threadIds: [second, first] });
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveValue('Second thread draft');
  await session.page.getByRole('button', { name: 'Switch thread', exact: true }).click();
  await session.page.locator('.thread-row').last().click();
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveValue('First thread draft');
});

test('background app is never visible or focused, but can draw and accept input', async ({ session }, testInfo) => {
  const state = () => session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map(w => ({
    visible: w.isVisible(), focused: w.isFocused(), devtools: w.webContents.isDevToolsOpened(),
  })));
  expect(await state()).toEqual([{ visible: false, focused: false, devtools: false }]);
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await session.page.getByRole('button', { name: 'New thread', exact: true }).click();
  await session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Background input works');
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveValue('Background input works');
  await testInfo.attach('hidden-agent-threads', { body: await session.page.screenshot(), contentType: 'image/png' });
  expect(await state()).toEqual([{ visible: false, focused: false, devtools: false }]);
});

test('thread transcripts and titles survive a hidden relaunch without leaking into another thread', async ({ session }) => {
  let page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({state:{},times:[],images:[]});
    PM.AgentThreadTitles.generate = async () => JSON.stringify({title:'First conversation request'});
    PM.CodexBridge.request = async () => ({text:JSON.stringify({summary:'A saved test reply',commands:[],artifacts:[],externalActions:[],notes:[]})});
  });
  await page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('First conversation request');
  await page.getByRole('button', {name:'Send message',exact:true}).click();
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  await expect(page.getByRole('button', {name:'Switch thread',exact:true})).toContainText('First conversation request');
  await page.getByRole('button', {name:'New thread',exact:true}).click();
  await expect(page.getByRole('log')).not.toContainText('First conversation request');
  await page.getByRole('button', {name:'Switch thread',exact:true}).click();
  await page.getByRole('option', {name:'First conversation request'}).click();
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  // Deliberately close immediately after typing: the native quit barrier must
  // flush the draft before its debounce timer fires.
  await page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('Last keystroke');
  await session.relaunch(); page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await expect(page.getByRole('log')).toContainText('First conversation request');
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  await expect(page.getByRole('textbox', {name:'Message Powermove agent',exact:true})).toHaveValue('Last keystroke');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
